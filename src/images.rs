//! Resolves the image URLs a document references.
//!
//! Markdown files usually point at images relative to themselves
//! (`![diagram](assets/diagram.png)`), which only makes sense once the
//! document's folder is known. Remote URLs load through GPUI's HTTP client
//! and `data:` URLs are decoded once and cached.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use gpui_kit::{Image, ImageFormat, ImageSource as GpuiImageSource, SharedUri};

use crate::{analysis::content_hash, links::percent_decode};

/// An image to add to the document (from pasted bytes or a dropped file).
#[derive(Debug, Clone)]
pub(crate) enum ImageSource {
    /// Pasted image data, with its file extension.
    Bytes(Vec<u8>, &'static str),
    File(PathBuf),
}

/// Whether `path` has an image file extension.
pub(crate) fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp"
            )
        })
}

/// Put `image` where the document can link it, returning the link path
/// (relative to `dir`) and alt text. Files already inside `dir` stay put.
pub(crate) fn store_image(
    image: &ImageSource,
    dir: &Path,
    folder: &str,
    stem: &str,
) -> std::io::Result<(String, String)> {
    if let ImageSource::File(path) = image
        && let Ok(relative) = path.strip_prefix(dir)
    {
        let alt = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        return Ok((link_path(relative), alt));
    }
    let target_dir = dir.join(folder);
    std::fs::create_dir_all(&target_dir)?;
    let (name, extension) = match image {
        ImageSource::Bytes(_, extension) => {
            let now = chrono_like_timestamp();
            (format!("{stem}-{now}"), (*extension).to_string())
        }
        ImageSource::File(path) => (
            path.file_stem().map_or_else(
                || "image".to_string(),
                |stem| stem.to_string_lossy().to_string(),
            ),
            path.extension().map_or_else(
                || "png".to_string(),
                |ext| ext.to_string_lossy().to_lowercase(),
            ),
        ),
    };
    let name: String = name
        .chars()
        .map(|ch| {
            if ch.is_whitespace() || "/\\:*?\"<>|".contains(ch) {
                '-'
            } else {
                ch
            }
        })
        .collect();
    // Never overwrite: number the name until it is free.
    let mut target = target_dir.join(format!("{name}.{extension}"));
    let mut counter = 2;
    while target.exists() {
        target = target_dir.join(format!("{name}-{counter}.{extension}"));
        counter += 1;
    }
    match image {
        ImageSource::Bytes(bytes, _) => std::fs::write(&target, bytes)?,
        ImageSource::File(path) => {
            std::fs::copy(path, &target)?;
        }
    }
    let relative = link_path(target.strip_prefix(dir).unwrap_or(&target));
    // Pasted images have no name worth describing them by.
    let alt = match image {
        ImageSource::Bytes(..) => "image".to_string(),
        ImageSource::File(_) => name,
    };
    Ok((relative, alt))
}

/// `path` as a Markdown link, which separates folders with `/` on every
/// system (Windows paths use `\\`).
pub(crate) fn link_path(path: &Path) -> String {
    let mut out = String::new();
    for (ix, component) in path.components().enumerate() {
        if ix > 0 {
            out.push('/');
        }
        out.push_str(&component.as_os_str().to_string_lossy());
    }
    out
}

/// `YYYYMMDD-HHMMSS` in UTC, for naming pasted images.
pub(crate) fn chrono_like_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    #[expect(
        clippy::cast_possible_wrap,
        clippy::as_conversions,
        reason = "Civil date Howard Hinnant algorithm days safely fits in i64"
    )]
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// Resolve `url` for display, relative to `base_dir` when it is a local path.
pub(crate) fn resolve(url: &SharedUri, base_dir: Option<&Path>) -> GpuiImageSource {
    let raw = url.as_ref();
    if let Some(data) = raw.strip_prefix("data:") {
        return decode_data_url(data).map_or_else(|| url.clone().into(), GpuiImageSource::Image);
    }
    match local_path(raw, base_dir) {
        Some(path) => GpuiImageSource::from(path),
        None => url.clone().into(),
    }
}

/// The file an image URL refers to, or `None` for remote URLs.
pub(crate) fn local_path(url: &str, base_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("file://") {
        return Some(PathBuf::from(percent_decode(path)));
    }
    if url.contains("://") || url.starts_with("//") || url.starts_with("mailto:") {
        return None;
    }
    // Drop a query or fragment; neither names part of a local file.
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let path = PathBuf::from(percent_decode(path));
    if path.is_absolute() {
        return Some(path);
    }
    Some(match base_dir {
        Some(dir) => dir.join(path),
        None => std::env::current_dir().ok()?.join(path),
    })
}

/// The bytes and MIME type of a `data:` URL (without the `data:` prefix).
fn data_url_bytes(data: &str) -> Option<(Vec<u8>, &str)> {
    let (meta, payload) = data.split_once(',')?;
    let mime = meta.split(';').next()?;
    let bytes = if meta.ends_with(";base64") {
        decode_base64(payload)?
    } else {
        percent_decode(payload).into_bytes()
    };
    Some((bytes, mime))
}

const DATA_URL_CACHE_LIMIT: usize = 128;

fn decode_data_url(data: &str) -> Option<Arc<Image>> {
    static CACHE: std::sync::LazyLock<Mutex<HashMap<u64, Arc<Image>>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    let key = content_hash(data);
    if let Some(image) = CACHE.lock().expect("data url cache poisoned").get(&key) {
        return Some(Arc::clone(image));
    }

    let (bytes, mime) = data_url_bytes(data)?;
    let format = ImageFormat::from_mime_type(mime)
        .or_else(|| (mime == "image/svg").then_some(ImageFormat::Svg))?;

    let image = Arc::new(Image::from_bytes(format, bytes));
    let mut cache = CACHE.lock().expect("data url cache poisoned");
    if cache.len() >= DATA_URL_CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(key, Arc::clone(&image));
    Some(image)
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        let v: u8 = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        };
        Some(u32::from(v))
    }

    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        if byte.is_ascii_whitespace() {
            continue;
        }
        buffer = (buffer << 6) | value(byte)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::as_conversions,
                reason = "lower 8 bits of shifted 32-bit buffer are intentionally extracted"
            )]
            let val = (buffer >> bits) as u8;
            out.push(val);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_paths_against_the_document() {
        let base = Path::new("/docs/guide");
        assert_eq!(
            local_path("img/a%20b.png?raw=1", Some(base)),
            Some(PathBuf::from("/docs/guide/img/a b.png"))
        );
        assert_eq!(
            local_path("/abs/x.png", Some(base)),
            Some(PathBuf::from("/abs/x.png"))
        );
        assert_eq!(
            local_path("file:///tmp/x.png", Some(base)),
            Some(PathBuf::from("/tmp/x.png"))
        );
        assert_eq!(local_path("https://example.com/x.png", Some(base)), None);
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64("aGk").unwrap(), b"hi");
        assert!(decode_base64("a$b").is_none());
    }

    #[test]
    fn links_use_forward_slashes() {
        let path = Path::new("assets").join("shots").join("a b.png");
        assert_eq!(link_path(&path), "assets/shots/a b.png");
    }

    #[test]
    fn stores_images_without_overwriting() {
        let dir = std::env::temp_dir().join(format!("downmark-images-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = ImageSource::Bytes(vec![1, 2, 3], "png");
        let (first, _) = store_image(&bytes, &dir, "assets", "My notes").unwrap();
        assert!(
            first.starts_with("assets/My-notes-")
                && Path::new(&first)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("png")),
            "{first}"
        );
        let outside = std::env::temp_dir().join(format!("downmark-pic-{}.png", std::process::id()));
        std::fs::write(&outside, [0]).unwrap();
        let (copied, alt) =
            store_image(&ImageSource::File(outside.clone()), &dir, "assets", "x").unwrap();
        assert!(copied.starts_with("assets/downmark-pic-"));
        assert!(alt.starts_with("downmark-pic-"));
        let (again, _) =
            store_image(&ImageSource::File(outside.clone()), &dir, "assets", "x").unwrap();
        assert_ne!(copied, again);
        // A file already next to the document is linked in place.
        let (linked, _) =
            store_image(&ImageSource::File(dir.join(&copied)), &dir, "assets", "x").unwrap();
        assert_eq!(linked, copied);
        let _ = std::fs::remove_file(outside);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn formats_timestamps() {
        let stamp = chrono_like_timestamp();
        assert_eq!(stamp.len(), 15);
        assert!(stamp.starts_with("20"));
    }
}
