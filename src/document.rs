//! The document being edited: where it lives and how it is read and written.

use std::{
    borrow::Cow,
    fs,
    io::{self, Write as _},
    path::Path,
};

/// Line ending convention detected when a file is opened, restored on save.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

impl LineEnding {
    pub(crate) fn detect(text: &str) -> Self {
        match text.find('\n') {
            Some(ix) if ix > 0 && text.as_bytes()[ix - 1] == b'\r' => LineEnding::CrLf,
            _ => LineEnding::Lf,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
        }
    }
}

/// Text read from disk, normalized to `\n` line endings for editing.
pub(crate) struct LoadedText {
    pub(crate) text: String,
    pub(crate) line_ending: LineEnding,
}

/// Read a UTF-8 text file. A byte-order mark is dropped.
pub(crate) fn load(path: &Path) -> io::Result<LoadedText> {
    let bytes = fs::read(path)?;
    let text = String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "The file isn’t valid UTF-8 text.",
        )
    })?;
    let stripped = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let line_ending = LineEnding::detect(stripped);
    let text = match line_ending {
        LineEnding::CrLf => stripped.replace("\r\n", "\n"),
        LineEnding::Lf => {
            if stripped.len() == text.len() {
                text
            } else {
                stripped.to_string()
            }
        }
    };
    Ok(LoadedText { text, line_ending })
}

/// Write `text` to `path` atomically: a temporary sibling file is written and
/// flushed, then renamed over the target, so a crash never leaves a
/// half-written document behind.
pub(crate) fn save(path: &Path, text: &str, line_ending: LineEnding) -> io::Result<()> {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let contents: Cow<'_, str> = match line_ending {
        LineEnding::CrLf => Cow::Owned(text.replace('\n', "\r\n")),
        LineEnding::Lf => Cow::Borrowed(text),
    };

    let dir = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing file name."))?;
    let temp = dir.join(format!(
        ".{}.downmark-{}-{}.tmp",
        file_name.to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));

    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        // Keep the permissions of the file being replaced.
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temp, metadata.permissions())?;
        }
        fs::rename(&temp, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Whether `path` looks like a Markdown document.
pub(crate) fn is_markdown_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdx" | "txt"
            )
        })
}

/// The name shown for a document in the title bar and dialogs.
pub(crate) fn display_name(path: Option<&Path>) -> String {
    path.and_then(Path::file_name).map_or_else(
        || "Untitled".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_line_endings() {
        assert_eq!(LineEnding::detect("a\r\nb"), LineEnding::CrLf);
        assert_eq!(LineEnding::detect("a\nb\r\n"), LineEnding::Lf);
        assert_eq!(LineEnding::detect("no newline"), LineEnding::Lf);
    }

    #[test]
    fn round_trips_crlf_and_bom() {
        let dir = std::env::temp_dir().join(format!("downmark-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("doc.md");
        fs::write(&path, "\u{feff}# Title\r\n\r\nBody\r\n").unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.text, "# Title\n\nBody\n");
        assert_eq!(loaded.line_ending, LineEnding::CrLf);

        save(&path, "# Title\n\nEdited\n", loaded.line_ending).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# Title\r\n\r\nEdited\r\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recognizes_markdown_extensions() {
        assert!(is_markdown_path(Path::new("README.md")));
        assert!(is_markdown_path(Path::new("notes.MARKDOWN")));
        assert!(!is_markdown_path(Path::new("image.png")));
        assert_eq!(display_name(None), "Untitled");
    }
}
