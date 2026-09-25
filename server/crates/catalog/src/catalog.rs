//! Read-only access to one library catalog (`lib_<id>.db`): a small connection pool,
//! atomic reload after re-import, and the browsing queries.

use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, RwLock};

use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, OptionalExtension, ToSql};

use crate::genres::genres;
use crate::model::*;
use crate::normalize::letter_of;
use crate::schema::CATALOG_SCHEMA_VERSION;
use crate::search::BookAttrs;

/// Errors of the read API.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("catalog schema version {found}, expected {expected}: re-import the library")]
    SchemaVersion { found: i64, expected: i64 },
    #[error("catalog file not found: {0}")]
    NotFound(PathBuf),
    #[error("invalid cursor")]
    BadCursor,
    /// The file was replaced by a re-import while this `Catalog` was still in use and needed a
    /// new connection; take a fresh `Arc<Catalog>` from the handle and retry.
    #[error("catalog was replaced by a re-import")]
    Stale,
}

pub type Result<T> = std::result::Result<T, CatalogError>;

/// Which books to list (exactly one per request, see `GET /libraries/:lib/books`).
#[derive(Debug, Clone)]
pub enum BookSelector {
    /// Books of an author. Order: series name, serno, title (books without series last).
    Author(i64),
    /// Books of a series. Order: serno, title.
    Series(i64),
    /// Books of a genre; a top-level group includes all its sub-genres. Order: date desc, title.
    Genre(u16),
    /// Books with `date >= since` (`YYYY-MM-DD`). Order: date desc, title.
    Since(String),
    /// A given set of book ids (shelves). Order: date desc, title.
    Ids(Vec<i64>),
}

/// Optional filters applied to every book list.
#[derive(Debug, Clone, Default)]
pub struct BookFilter {
    /// Only these languages (empty = all).
    pub langs: Vec<String>,
    /// Only this extension (`fb2`, `epub`, …).
    pub ext: Option<String>,
    /// Include books marked deleted (API `deleted=1`). Default: hidden.
    pub include_deleted: bool,
    /// Only books matching these words (prefix per word over title, authors, series and
    /// keywords, like search). `None` or no usable words = no filter.
    pub q: Option<String>,
}

/// Pagination: `cursor` is the opaque `nextCursor` of the previous page.
#[derive(Debug, Clone)]
pub struct Page {
    pub cursor: Option<String>,
    pub limit: usize,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            cursor: None,
            limit: 2000,
        }
    }
}

pub(crate) fn id_array(ids: impl IntoIterator<Item = i64>) -> Rc<Vec<Value>> {
    Rc::new(ids.into_iter().map(Value::Integer).collect())
}

pub(crate) fn text_array<S: AsRef<str>>(v: impl IntoIterator<Item = S>) -> Rc<Vec<Value>> {
    Rc::new(
        v.into_iter()
            .map(|s| Value::Text(s.as_ref().to_string()))
            .collect(),
    )
}

const MAX_IDLE: usize = 8;

/// Genres with at least this many live books are listed by walking the date index.
const BIG_GENRE_MIN_BOOKS: i64 = 20_000;

/// Selections whose total is counted over [`BookAttrs`].
pub(crate) enum CountSel {
    Genres(Vec<u16>),
    /// `date >= d`
    Since(String),
    /// `date > d`
    Newer(String),
}

/// An open catalog: immutable data, pooled read-only connections.
/// Cheap to share as `Arc<Catalog>`; old instances keep working after a reload
/// (their file descriptors point at the replaced file).
pub struct Catalog {
    path: PathBuf,
    idle: Mutex<Vec<Connection>>,
    stats: LibraryStats,
    attrs: Mutex<Option<Arc<BookAttrs>>>,
    big_genre: i64,
}

/// A connection checked out of the pool; returned on drop.
pub struct PooledConn<'a> {
    cat: &'a Catalog,
    conn: Option<Connection>,
}

impl Deref for PooledConn<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("connection present until drop")
    }
}

impl Drop for PooledConn<'_> {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take() {
            let mut idle = self.cat.idle.lock().unwrap_or_else(|e| e.into_inner());
            if idle.len() < MAX_IDLE {
                idle.push(c);
            }
        }
    }
}

/// Open a read-only catalog connection with the runtime pragmas.
pub fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.execute_batch(
        "PRAGMA query_only=1; PRAGMA cache_size=-65536; PRAGMA temp_store=MEMORY; PRAGMA mmap_size=1073741824;",
    )?;
    rusqlite::vtab::array::load_module(&conn)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.set_prepared_statement_cache_capacity(64);
    Ok(conn)
}

impl Catalog {
    /// Open `lib_<id>.db`, check its schema version and read its stats.
    pub fn open(path: impl AsRef<Path>) -> Result<Catalog> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(CatalogError::NotFound(path));
        }
        let conn = open_read_only(&path)?;
        let meta: HashMap<String, String> = {
            let mut st = conn.prepare("SELECT key, value FROM meta")?;
            st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        let int = |k: &str| meta.get(k).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        let found = int("schema_version");
        if found != CATALOG_SCHEMA_VERSION {
            return Err(CatalogError::SchemaVersion {
                found,
                expected: CATALOG_SCHEMA_VERSION,
            });
        }
        let opt = |k: &str| meta.get(k).filter(|v| !v.is_empty()).cloned();
        let stats = LibraryStats {
            book_count: int("book_count"),
            live_book_count: int("live_book_count"),
            author_count: int("author_count"),
            series_count: int("series_count"),
            imported_at: opt("imported_at"),
            catalog_version: int("catalog_version"),
            inpx_version: opt("inpx_version"),
            source_inpx: opt("source_inpx"),
            collection_name: opt("collection_name"),
            first_author_only: int("first_author_only") != 0,
            skip_deleted: int("skip_deleted") != 0,
        };
        Ok(Catalog {
            path,
            idle: Mutex::new(vec![conn]),
            stats,
            attrs: Mutex::new(None),
            big_genre: BIG_GENRE_MIN_BOOKS,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Override the genre size above which the date-index strategy is used (tests/benchmarks).
    #[doc(hidden)]
    pub fn set_big_genre_threshold(&mut self, books: i64) {
        self.big_genre = books;
    }

    /// Check out a pooled connection (opens a new one when the pool is empty).
    pub fn conn(&self) -> Result<PooledConn<'_>> {
        let pooled = self.idle.lock().unwrap_or_else(|e| e.into_inner()).pop();
        let conn = match pooled {
            Some(c) => c,
            None => {
                let c = open_read_only(&self.path)?;
                // A re-import may have renamed a new file over ours: never mix two catalogs.
                let v: Option<String> = c
                    .query_row(
                        "SELECT value FROM meta WHERE key='catalog_version'",
                        [],
                        |r| r.get(0),
                    )
                    .optional()?;
                if v.and_then(|v| v.parse::<i64>().ok()) != Some(self.stats.catalog_version) {
                    return Err(CatalogError::Stale);
                }
                c
            }
        };
        Ok(PooledConn {
            cat: self,
            conn: Some(conn),
        })
    }

    /// Counts and import metadata (cached at open).
    pub fn stats(&self) -> &LibraryStats {
        &self.stats
    }

    pub fn catalog_version(&self) -> i64 {
        self.stats.catalog_version
    }

    /// Authors with at least one live book, ordered by `sort_key, id` with the names that do not
    /// start with a letter (`#` group: digits, symbols) moved to the end, plus the letter index.
    pub fn authors(&self) -> Result<NameList> {
        self.name_list(
            "SELECT id, name, book_count, sort_key FROM author WHERE book_count > 0 ORDER BY sort_key, id",
            "author",
        )
    }

    /// Series with at least one live book, in the same order as [`authors`](Self::authors).
    pub fn series_list(&self) -> Result<NameList> {
        self.name_list(
            "SELECT id, name, book_count, sort_key FROM series WHERE book_count > 0 ORDER BY sort_key, id",
            "series",
        )
    }

    fn name_list(&self, sql: &str, kind: &str) -> Result<NameList> {
        let conn = self.conn()?;
        let cap = match kind {
            "author" => self.stats.author_count,
            _ => self.stats.series_count,
        } as usize;
        let mut rows = Vec::with_capacity(cap);
        let mut other = Vec::new();
        let mut letters: Vec<(String, i64, i64)> = Vec::new();
        let mut st = conn.prepare_cached(sql)?;
        let mut q = st.query([])?;
        while let Some(r) = q.next()? {
            let row: (i64, String, i64) = (r.get(0)?, r.get(1)?, r.get(2)?);
            let key: String = r.get(3)?;
            let letter = letter_of(&key);
            if letter == "#" {
                other.push(row);
                continue;
            }
            match letters.last_mut() {
                Some(l) if l.0 == letter => l.1 += 1,
                _ => letters.push((letter, 1, rows.len() as i64)),
            }
            rows.push(row);
        }
        if !other.is_empty() {
            letters.push(("#".to_string(), other.len() as i64, rows.len() as i64));
            rows.extend(other);
        }
        Ok(NameList { rows, letters })
    }

    /// One author by id.
    pub fn author(&self, id: i64) -> Result<Option<NameCount>> {
        let conn = self.conn()?;
        let mut st = conn.prepare_cached("SELECT id, name, book_count FROM author WHERE id=?1")?;
        Ok(st
            .query_row([id], |r| {
                Ok(NameCount {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    count: r.get(2)?,
                })
            })
            .optional()?)
    }

    /// One series by id (with its main authors string).
    pub fn series(&self, id: i64) -> Result<Option<SeriesHit>> {
        let conn = self.conn()?;
        let mut st =
            conn.prepare_cached("SELECT id, name, book_count, authors FROM series WHERE id=?1")?;
        Ok(st
            .query_row([id], |r| {
                Ok(SeriesHit {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    count: r.get(2)?,
                    authors: r.get(3)?,
                })
            })
            .optional()?)
    }

    /// All genres (322) with this library's counts; zero-count genres included.
    pub fn genres(&self, lang: &str) -> Result<Vec<GenreCount>> {
        let conn = self.conn()?;
        let mut st = conn.prepare_cached("SELECT genre_id, count FROM genre_count")?;
        let counts: HashMap<u16, i64> = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(genres()
            .all()
            .iter()
            .map(|g| GenreCount {
                id: g.id,
                name: g.localized(lang).to_string(),
                parent: g.parent,
                count: counts.get(&g.id).copied().unwrap_or(0),
            })
            .collect())
    }

    /// `[(lang, count)]` of non-deleted books, most frequent first.
    pub fn languages(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.conn()?;
        let mut st =
            conn.prepare_cached("SELECT lang, count FROM lang_count ORDER BY count DESC, lang")?;
        Ok(st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Number of non-deleted books with `date > after` (for "new since last visit").
    /// `after` may be a date or an RFC 3339 timestamp (only the date part is used).
    /// Counted in memory over [`BookAttrs`] (loads them on first use).
    pub fn count_newer_than(&self, after: &str) -> Result<i64> {
        Ok(self
            .attrs()?
            .count(&CountSel::Newer(after.to_string()), &BookFilter::default()))
    }

    /// A page of books for one selector, with filters and cursor pagination.
    pub fn books(&self, sel: &BookSelector, filter: &BookFilter, page: &Page) -> Result<BookPage> {
        let offset: usize = match &page.cursor {
            None => 0,
            Some(c) if c.is_empty() => 0,
            Some(c) => c.parse().map_err(|_| CatalogError::BadCursor)?,
        };
        let limit = page.limit.max(1);

        let date_order = "b.date DESC, b.sort_key, b.id";
        let conn = self.conn()?;
        // Large result sets (genre, since) are counted in memory rather than by SQL.
        let mut count_attrs: Option<CountSel> = None;
        let (from, order, sel_param): (String, &str, Box<dyn ToSql>) = match sel {
            BookSelector::Author(id) => (
                "FROM book_author ba JOIN book b ON b.id=ba.book_id LEFT JOIN series s ON s.id=b.series_id \
                 WHERE ba.author_id=?1"
                    .into(),
                "s.sort_key IS NULL, s.sort_key, b.series_id, b.serno IS NULL, b.serno, b.sort_key, b.id",
                Box::new(*id),
            ),
            BookSelector::Series(id) => {
                ("FROM book b WHERE b.series_id=?1".into(), "b.serno IS NULL, b.serno, b.sort_key, b.id", Box::new(*id))
            }
            BookSelector::Genre(g) => {
                let ids = genres().with_descendants(*g);
                let live: i64 = conn
                    .prepare_cached("SELECT count FROM genre_count WHERE genre_id=?1")?
                    .query_row([*g as i64], |r| r.get(0))
                    .optional()?
                    .unwrap_or(0);
                let from = if live >= self.big_genre {
                    // Large genre: walk the date index newest-first and stop after one page,
                    // instead of sorting every book of the genre.
                    "FROM book b INDEXED BY book_date WHERE EXISTS \
                     (SELECT 1 FROM book_genre bg WHERE bg.book_id=b.id AND bg.genre_id IN rarray(?1))"
                        .to_string()
                } else if ids.len() == 1 {
                    "FROM book_genre bg JOIN book b ON b.id=bg.book_id WHERE bg.genre_id IN rarray(?1)".to_string()
                } else {
                    "FROM book b WHERE b.id IN (SELECT book_id FROM book_genre WHERE genre_id IN rarray(?1))".to_string()
                };
                count_attrs = Some(CountSel::Genres(ids.clone()));
                (from, date_order, Box::new(id_array(ids.into_iter().map(i64::from))))
            }
            BookSelector::Since(d) => {
                count_attrs = Some(CountSel::Since(d.clone()));
                ("FROM book b WHERE b.date >= ?1".into(), date_order, Box::new(d.clone()))
            }
            BookSelector::Ids(ids) => {
                ("FROM book b WHERE b.id IN rarray(?1)".into(), date_order, Box::new(id_array(ids.iter().copied())))
            }
        };

        let mut params: Vec<Box<dyn ToSql>> = vec![sel_param];
        let mut where_extra = String::new();
        if !filter.include_deleted {
            where_extra.push_str(" AND b.deleted=0");
        }
        if !filter.langs.is_empty() {
            params.push(Box::new(text_array(&filter.langs)));
            where_extra.push_str(&format!(" AND b.lang IN rarray(?{})", params.len()));
        }
        if let Some(ext) = &filter.ext {
            params.push(Box::new(ext.to_lowercase()));
            where_extra.push_str(&format!(" AND b.ext=?{}", params.len()));
        }
        let fts = filter.q.as_deref().and_then(crate::search::fts_query);
        if let Some(f) = &fts {
            params.push(Box::new(f.clone()));
            where_extra.push_str(&format!(
                " AND b.id IN (SELECT rowid FROM book_fts WHERE book_fts MATCH ?{})",
                params.len()
            ));
            // the in-memory counter knows nothing about the text filter
            count_attrs = None;
        }

        let sql = format!(
            "SELECT b.id {from}{where_extra} ORDER BY {order} LIMIT ?{} OFFSET ?{}",
            params.len() + 1,
            params.len() + 2
        );
        let ids: Vec<i64> = {
            let mut st = conn.prepare_cached(&sql)?;
            let mut all: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref()).collect();
            let lim = (limit + 1) as i64;
            let off = offset as i64;
            all.push(&lim);
            all.push(&off);
            st.query_map(all.as_slice(), |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let has_more = ids.len() > limit;
        let ids = &ids[..ids.len().min(limit)];
        let total = if offset == 0 && !has_more {
            ids.len() as i64
        } else if let Some(sel) = count_attrs {
            self.attrs()?.count(&sel, filter)
        } else {
            let mut st = conn.prepare_cached(&format!("SELECT count(*) {from}{where_extra}"))?;
            let all: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref()).collect();
            st.query_row(all.as_slice(), |r| r.get(0))?
        };
        let books = load_books(&conn, ids)?;
        Ok(BookPage {
            books,
            next_cursor: has_more.then(|| (offset + limit).to_string()),
            total,
        })
    }

    /// Books by id, in the given order (unknown ids are skipped). No filters.
    pub fn books_by_ids(&self, ids: &[i64]) -> Result<Vec<Book>> {
        let conn = self.conn()?;
        load_books(&conn, ids)
    }

    /// One book with location fields.
    pub fn book(&self, id: i64) -> Result<Option<BookDetail>> {
        let conn = self.conn()?;
        let Some(book) = load_books(&conn, &[id])?.pop() else {
            return Ok(None);
        };
        let mut st = conn.prepare_cached(
            "SELECT file, archive, folder, keywords, lib_id, stars, arch_offset, arch_csize, arch_method FROM book WHERE id=?1",
        )?;
        Ok(Some(st.query_row([id], |r| {
            Ok(BookDetail {
                book,
                file: r.get(0)?,
                archive: r.get(1)?,
                folder: r.get(2)?,
                keywords: r.get(3)?,
                lib_id: r.get(4)?,
                stars: r.get(5)?,
                arch_offset: r.get(6)?,
                arch_csize: r.get(7)?,
                arch_method: r.get(8)?,
            })
        })?))
    }

    /// Resolve `book_key`s to ids: `[(key, id)]` for the keys present in this catalog.
    pub fn ids_by_keys(&self, keys: &[String]) -> Result<Vec<(String, i64)>> {
        let conn = self.conn()?;
        let mut st =
            conn.prepare_cached("SELECT book_key, id FROM book WHERE book_key IN rarray(?1)")?;
        Ok(st
            .query_map([text_array(keys)], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Book ids → `book_key`s: `[(id, key)]` for ids present.
    pub fn keys_by_ids(&self, ids: &[i64]) -> Result<Vec<(i64, String)>> {
        let conn = self.conn()?;
        let mut st = conn.prepare_cached("SELECT id, book_key FROM book WHERE id IN rarray(?1)")?;
        Ok(st
            .query_map([id_array(ids.iter().copied())], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Per-book attributes used by search filtering/facets; loaded on first use
    /// (~1 s on 600k books). Call from a background task after open to warm up.
    pub fn attrs(&self) -> Result<Arc<BookAttrs>> {
        let mut guard = self.attrs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = guard.as_ref() {
            return Ok(a.clone());
        }
        let conn = self.conn()?;
        let a = Arc::new(BookAttrs::load(&conn)?);
        *guard = Some(a.clone());
        Ok(a)
    }
}

/// Load full `Book` rows for `ids` (in that order) with 3 queries: books, authors, genres.
pub(crate) fn load_books(conn: &Connection, ids: &[i64]) -> Result<Vec<Book>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let arr = id_array(ids.iter().copied());
    let mut by_id: HashMap<i64, Book> = HashMap::with_capacity(ids.len());
    {
        let mut st = conn.prepare_cached(
            "SELECT b.id, b.book_key, b.title, b.series_id, s.name, b.serno, b.lang, b.ext, b.size, b.date, b.deleted \
             FROM book b LEFT JOIN series s ON s.id=b.series_id WHERE b.id IN rarray(?1)",
        )?;
        let mut q = st.query([arr.clone()])?;
        while let Some(r) = q.next()? {
            let id: i64 = r.get(0)?;
            let series_id: Option<i64> = r.get(3)?;
            let series = match series_id {
                Some(sid) => Some(SeriesRef {
                    id: sid,
                    name: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                }),
                None => None,
            };
            by_id.insert(
                id,
                Book {
                    id,
                    key: r.get(1)?,
                    title: r.get(2)?,
                    authors: Vec::new(),
                    series,
                    serno: r.get(5)?,
                    genres: Vec::new(),
                    lang: r.get(6)?,
                    ext: r.get(7)?,
                    size: r.get(8)?,
                    date: r.get(9)?,
                    deleted: r.get::<_, i64>(10)? != 0,
                },
            );
        }
    }
    {
        let mut st = conn.prepare_cached(
            "SELECT ba.book_id, a.id, a.name FROM book_author ba JOIN author a ON a.id=ba.author_id \
             WHERE ba.book_id IN rarray(?1) ORDER BY ba.book_id, ba.pos",
        )?;
        let mut q = st.query([arr.clone()])?;
        while let Some(r) = q.next()? {
            if let Some(b) = by_id.get_mut(&r.get::<_, i64>(0)?) {
                b.authors.push(AuthorRef {
                    id: r.get(1)?,
                    name: r.get(2)?,
                });
            }
        }
    }
    {
        let mut st = conn.prepare_cached(
            "SELECT book_id, genre_id FROM book_genre WHERE book_id IN rarray(?1)",
        )?;
        let mut q = st.query([arr])?;
        while let Some(r) = q.next()? {
            if let Some(b) = by_id.get_mut(&r.get::<_, i64>(0)?) {
                b.genres.push(r.get(1)?);
            }
        }
    }
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

/// A per-library slot holding the current [`Catalog`], swapped atomically by [`reload`](Self::reload).
///
/// Requests call [`get`](Self::get) once and keep the `Arc<Catalog>` for the whole request,
/// so a reload never mixes two catalog versions inside one response.
pub struct CatalogHandle {
    path: PathBuf,
    current: RwLock<Option<Arc<Catalog>>>,
}

impl CatalogHandle {
    /// Create a handle for `path`; opens the catalog if the file exists and is valid
    /// (errors are swallowed here — check [`get`](Self::get) / call [`reload`](Self::reload)).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let cat = Catalog::open(&path).ok().map(Arc::new);
        CatalogHandle {
            path,
            current: RwLock::new(cat),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The current catalog, `None` if never imported (or the file is unusable).
    pub fn get(&self) -> Option<Arc<Catalog>> {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Re-open the file (after the importer renamed a new one into place) and swap it in.
    /// On error the previous catalog stays active.
    pub fn reload(&self) -> Result<Arc<Catalog>> {
        let cat = Arc::new(Catalog::open(&self.path)?);
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = Some(cat.clone());
        Ok(cat)
    }

    /// Drop the current catalog (e.g. before deleting the library).
    pub fn close(&self) {
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = None;
    }
}
