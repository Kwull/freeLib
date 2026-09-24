//! freelib-fb2conv: FB2 metadata reading and FB2 → EPUB 3 / KEPUB conversion for the freeLib
//! web edition. Port of the Qt `fb2mobi` converter. See `README.md` for details.
//!
//! ```no_run
//! use freelib_fb2conv::{Assets, ConvertOptions, fb2_to_epub, read_info, to_kepub};
//! let fb2 = std::fs::read("book.fb2").unwrap();
//! let info = read_info(&fb2).unwrap();
//! let epub = fb2_to_epub(&fb2, &ConvertOptions::default(), Assets::shared()).unwrap();
//! let kepub = to_kepub(&epub).unwrap();
//! ```

mod assets;
mod convert;
mod cover;
mod decode;
mod dom;
mod epub;
mod epub_info;
mod hyph;
mod images;
mod info;
mod kepub;
pub mod limit;
mod names;
mod options;
mod xml;

pub use assets::{Assets, FontFace, FontFamily};
pub use convert::{fb2_to_epub, join_to_epub};
pub use cover::generate_cover;
pub use epub_info::read_info_epub;
pub use info::{BookInfo, CoverImage, Person, read_info, sanitize_html};
pub use kepub::{kepubify_xhtml, to_kepub};
pub use names::{NameFields, expand as expand_template, file_name, transliteration};
pub use options::{ConvertOptions, CreateCover, Footnotes, Hyphenate, TocPlacement};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid book: {0}")]
    Format(String),
    #[error("zip: {0}")]
    Zip(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Reads metadata of any supported input: FB2 (plain or zipped) or EPUB.
pub fn read_info_any(bytes: &[u8]) -> Result<BookInfo> {
    if decode::is_zip(bytes) {
        // EPUB: a zip with a META-INF/container.xml
        if let Ok(z) = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            && z.index_for_name("META-INF/container.xml").is_some()
        {
            return read_info_epub(bytes);
        }
    }
    read_info(bytes)
}
