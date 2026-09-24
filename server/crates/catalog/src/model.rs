//! Plain result types returned by the read API (serde, camelCase, matching docs/web/API.md
//! where sensible). User-specific fields (`rating`, `shelves`) are added by the server.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorRef {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRef {
    pub id: i64,
    pub name: String,
}

/// API `Book` minus `rating` / `shelves`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub id: i64,
    /// `book_key`: stable across re-imports.
    pub key: String,
    pub title: String,
    /// In INPX order; the first one is the book's first author.
    pub authors: Vec<AuthorRef>,
    pub series: Option<SeriesRef>,
    pub serno: Option<i64>,
    pub genres: Vec<u16>,
    pub lang: String,
    pub ext: String,
    pub size: i64,
    pub date: String,
    pub deleted: bool,
}

/// A book with its storage location and remaining INPX fields.
/// The server adds annotation / cover / formats to build API `BookDetail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDetail {
    #[serde(flatten)]
    pub book: Book,
    /// INPX FILE (no extension).
    pub file: String,
    /// Zip archive relative to the library folder, `""` for a plain file.
    pub archive: String,
    /// Raw INPX FOLDER value (`""` when absent); for plain files the sub-folder of the file.
    pub folder: String,
    pub keywords: String,
    pub lib_id: Option<i64>,
    /// INPX STARS (0..5), not the user rating.
    pub stars: i64,
    /// Byte offset of the zip local file header, when resolved.
    pub arch_offset: Option<i64>,
    pub arch_csize: Option<i64>,
    /// Zip compression method (0 = stored, 8 = deflate).
    pub arch_method: Option<i64>,
}

impl BookDetail {
    /// Entry name inside the archive (or file name on disk): `<file>.<ext>`.
    pub fn entry_name(&self) -> String {
        if self.book.ext.is_empty() { self.file.clone() } else { format!("{}.{}", self.file, self.book.ext) }
    }

    /// Path relative to the library folder: `<archive>` for zipped books,
    /// `<folder>/<file>.<ext>` (or `<file>.<ext>`) for plain files.
    pub fn relative_path(&self) -> String {
        if !self.archive.is_empty() {
            self.archive.clone()
        } else if !self.folder.is_empty() {
            format!("{}/{}", self.folder.trim_end_matches('/'), self.entry_name())
        } else {
            self.entry_name()
        }
    }

    /// API display string `"<archive> / <file>.<ext>"` (just the file name for plain files).
    pub fn display_file(&self) -> String {
        if self.archive.is_empty() { self.relative_path() } else { format!("{} / {}", self.archive, self.entry_name()) }
    }
}

/// A page of books.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookPage {
    pub books: Vec<Book>,
    pub next_cursor: Option<String>,
    pub total: i64,
}

/// Compact list for authors / series: `rows = [[id, name, count], …]` in sort-key order,
/// `letters = [[letter, count, firstRowIndex], …]`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NameList {
    pub rows: Vec<(i64, String, i64)>,
    pub letters: Vec<(String, i64, i64)>,
}

/// An author or series with its non-deleted book count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NameCount {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

/// Series search hit: `authors` is a display string of the series' main author(s).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesHit {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub authors: String,
}

/// API `Genre`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreCount {
    pub id: u16,
    pub name: String,
    pub parent: u16,
    /// Non-deleted books; for a top-level group: distinct books in the group.
    pub count: i64,
}

/// Library statistics and import metadata (from the `meta` table).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    /// All books in the catalog, including deleted ones.
    pub book_count: i64,
    /// Books with `deleted = 0`.
    pub live_book_count: i64,
    pub author_count: i64,
    pub series_count: i64,
    pub imported_at: Option<String>,
    pub catalog_version: i64,
    pub inpx_version: Option<String>,
    pub source_inpx: Option<String>,
    /// First line of `collection.info` (collection name), if any.
    pub collection_name: Option<String>,
    pub first_author_only: bool,
    pub skip_deleted: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Facets {
    pub genre: Vec<(u16, i64)>,
    pub lang: Vec<(String, i64)>,
    pub ext: Vec<(String, i64)>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub took_ms: u64,
    pub authors: Vec<NameCount>,
    pub series: Vec<SeriesHit>,
    pub books: Vec<Book>,
    /// Number of matching books after filters.
    pub total: i64,
    pub facets: Facets,
}
