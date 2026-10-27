//! A sealed Typst environment used for math rendering.
//!
//! The compiler sees one source file, the fonts bundled with Typst, and any
//! files the caller hands over in memory (images, icons). It cannot read the
//! file system or reach the network.

use std::{collections::HashMap, sync::LazyLock};

use typst::{
    Library, LibraryExt as _, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Duration},
    syntax::{FileId, Source},
    text::{Font, FontBook},
    utils::LazyHash,
};
use typst_layout::PagedDocument;

static LIBRARY: LazyLock<LazyHash<Library>> = LazyLock::new(|| LazyHash::new(Library::default()));

/// The fonts a world offers, with the book the compiler selects them from.
pub(crate) struct FontSet {
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}

fn bundled_fonts() -> (FontBook, Vec<Font>) {
    let fonts: Vec<Font> = typst_assets::fonts()
        .flat_map(|data| Font::iter(Bytes::new(data)))
        .collect();
    let book = FontBook::from_fonts(&fonts);
    (book, fonts)
}

/// Only the fonts bundled with Typst: fast to set up, identical everywhere.
pub(crate) static BUNDLED_FONTS: LazyLock<FontSet> = LazyLock::new(|| {
    let (book, fonts) = bundled_fonts();
    FontSet {
        book: LazyHash::new(book),
        fonts,
    }
});

/// Files available to the document, keyed by their path from the project
/// root without a leading slash (`image("/img-1.png")` → `img-1.png`).
pub(crate) type VirtualFiles = HashMap<String, Bytes>;

struct SandboxWorld {
    source: Source,
    files: VirtualFiles,
    fonts: &'static FontSet,
}

impl World for SandboxWorld {
    fn library(&self) -> &LazyHash<Library> {
        &LIBRARY
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.fonts.book
    }

    fn main(&self) -> FileId {
        self.source.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.source.id() {
            Ok(self.source.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        let path = id.vpath().get_without_slash();
        self.files.get(path).cloned().ok_or(FileError::AccessDenied)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.fonts.get(index).cloned()
    }

    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}

/// Compile Typst `source` into pages. The error is the first diagnostic,
/// worded for the user.
pub(crate) fn compile(
    source: String,
    files: VirtualFiles,
    fonts: &'static FontSet,
) -> Result<PagedDocument, String> {
    let world = SandboxWorld {
        source: Source::detached(source),
        files,
        fonts,
    };
    let result = typst::compile::<PagedDocument>(&world).output;
    // Typst memoizes layout across compilations; keep only recent entries so
    // many distinct inputs don't accumulate.
    comemo::evict(30);
    result.map_err(|errors| {
        errors.first().map_or_else(
            || "unknown error".to_string(),
            |error| error.message.to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_virtual_files_only() {
        let mut files = VirtualFiles::new();
        files.insert(
            "dot.svg".into(),
            Bytes::new(
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#
                    .to_vec(),
            ),
        );
        assert!(compile("#image(\"/dot.svg\")".into(), files, &BUNDLED_FONTS).is_ok());
        assert!(
            compile(
                "#image(\"/etc/passwd\")".into(),
                VirtualFiles::new(),
                &BUNDLED_FONTS
            )
            .is_err()
        );
    }
}
