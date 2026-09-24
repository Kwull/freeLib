//! Thin seam over `freelib-fb2conv`, so the rest of the server does not depend on its internals.

use freelib_fb2conv::{Assets, BookInfo, ConvertOptions, NameFields};

pub trait Converter: Send + Sync {
    /// Metadata, annotation and cover of an FB2 (plain or zipped) or EPUB.
    fn read_info(&self, bytes: &[u8]) -> anyhow::Result<BookInfo>;
    fn fb2_to_epub(&self, fb2: &[u8], opts: &ConvertOptions) -> anyhow::Result<Vec<u8>>;
    /// Several FB2 books (one series) → one EPUB.
    fn join_to_epub(
        &self,
        books: &[&[u8]],
        opts: &ConvertOptions,
        title: Option<&str>,
    ) -> anyhow::Result<Vec<u8>>;
    fn to_kepub(&self, epub: &[u8]) -> anyhow::Result<Vec<u8>>;
    /// Relative file name (may contain `/`) without extension.
    fn file_name(&self, template: &str, fields: &NameFields, transliterate: bool) -> String;
    fn font_names(&self) -> Vec<String>;
    /// Bumped when the converter output changes (part of the conversion cache key).
    fn version(&self) -> &'static str;
}

pub struct Fb2Conv;

impl Converter for Fb2Conv {
    fn read_info(&self, bytes: &[u8]) -> anyhow::Result<BookInfo> {
        Ok(freelib_fb2conv::read_info_any(bytes)?)
    }
    fn fb2_to_epub(&self, fb2: &[u8], opts: &ConvertOptions) -> anyhow::Result<Vec<u8>> {
        Ok(freelib_fb2conv::fb2_to_epub(fb2, opts, Assets::shared())?)
    }
    fn join_to_epub(
        &self,
        books: &[&[u8]],
        opts: &ConvertOptions,
        title: Option<&str>,
    ) -> anyhow::Result<Vec<u8>> {
        Ok(freelib_fb2conv::join_to_epub(
            books,
            opts,
            Assets::shared(),
            title,
        )?)
    }
    fn to_kepub(&self, epub: &[u8]) -> anyhow::Result<Vec<u8>> {
        Ok(freelib_fb2conv::to_kepub(epub)?)
    }
    fn file_name(&self, template: &str, fields: &NameFields, transliterate: bool) -> String {
        freelib_fb2conv::file_name(template, fields, transliterate)
    }
    fn font_names(&self) -> Vec<String> {
        Assets::shared().font_names()
    }
    fn version(&self) -> &'static str {
        concat!("fb2conv-", env!("CARGO_PKG_VERSION"), "-1")
    }
}
