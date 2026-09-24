//! Migration from the Qt desktop database (`freeLib.sqlite`).
//!
//! [`read_qt_database`] extracts library definitions, book-level tags (→ shelves) and book
//! ratings (`book.star` → ratings) into a plain [`QtMigration`]; [`QtMigration::apply`]
//! writes them into `app.db` for one user. Books are identified by `book_key`, computed from
//! the old row exactly like the importer does (`lib:<id_inlib>` or `file:<archive>/<file>.<ext>`),
//! so shelves and ratings attach to the books once the libraries are (re-)imported.
//!
//! Author and series tags have no equivalent in the web edition; they are counted and skipped.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

use crate::inpx::file_key;
use crate::ImportError;

/// A library of the Qt app (`lib` table).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QtLibrary {
    pub qt_id: i64,
    pub name: String,
    /// As stored by Qt: may be relative to the Qt app folder or contain `%`-placeholders.
    pub path: String,
    pub inpx: Option<String>,
    pub first_author_only: bool,
    pub skip_deleted: bool,
    pub version: Option<String>,
}

/// A Qt tag with its tagged books → one shelf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QtShelf {
    pub tag_id: i64,
    pub name: String,
    /// `#rrggbb`, chosen from a fixed palette by tag id.
    pub color: String,
    /// `(qt library id, book_key)`
    pub books: Vec<(i64, String)>,
}

/// Everything migrated from one `freeLib.sqlite`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QtMigration {
    pub libraries: Vec<QtLibrary>,
    /// Only tags that have at least one tagged book.
    pub shelves: Vec<QtShelf>,
    /// `(qt library id, book_key, rating 1..5)`
    pub ratings: Vec<(i64, String, i64)>,
    pub skipped_author_tags: u64,
    pub skipped_series_tags: u64,
}

const PALETTE: &[&str] = &["#e5484d", "#f76b15", "#ffc53d", "#46a758", "#12a594", "#0090ff", "#8e4ec6", "#d6409f"];

fn has_table(c: &Connection, t: &str) -> rusqlite::Result<bool> {
    c.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1", [t], |_| Ok(()))
        .optional()
        .map(|r| r.is_some())
}

fn has_column(c: &Connection, t: &str, col: &str) -> rusqlite::Result<bool> {
    let mut st = c.prepare(&format!("PRAGMA table_info(\"{t}\")"))?;
    let cols: Vec<String> = st.query_map([], |r| r.get(1))?.collect::<rusqlite::Result<_>>()?;
    Ok(cols.iter().any(|c| c.eq_ignore_ascii_case(col)))
}

/// `book_key` of a Qt `book` row (the Qt `archive` column holds the `.inp` name or FOLDER value).
pub fn qt_book_key(id_inlib: Option<i64>, archive: &str, file: &str, format: &str) -> String {
    if let Some(id) = id_inlib.filter(|&i| i > 0) {
        return format!("lib:{id}");
    }
    let mut a = archive.trim().replace('\\', "/");
    if a.to_ascii_lowercase().ends_with(".inp") {
        a = format!("{}.zip", &a[..a.len() - 4]);
    }
    file_key(&a, file.trim(), &format.trim().trim_start_matches('.').to_lowercase())
}

/// Read libraries, book tags and ratings from a Qt `freeLib.sqlite` (opened read-only).
pub fn read_qt_database(path: &Path) -> Result<QtMigration, ImportError> {
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    let mut m = QtMigration::default();

    if has_table(&c, "lib")? {
        let fa = if has_column(&c, "lib", "firstAuthor")? { "firstAuthor" } else { "0" };
        let wd = if has_column(&c, "lib", "woDeleted")? { "woDeleted" } else { "0" };
        let ver = if has_column(&c, "lib", "version")? { "version" } else { "NULL" };
        let mut st = c.prepare(&format!("SELECT id, name, path, inpx, {fa}, {wd}, {ver} FROM lib ORDER BY id"))?;
        m.libraries = st
            .query_map([], |r| {
                Ok(QtLibrary {
                    qt_id: r.get(0)?,
                    name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    path: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    inpx: r.get::<_, Option<String>>(3)?.filter(|s| !s.trim().is_empty()),
                    first_author_only: r.get::<_, Option<i64>>(4)?.unwrap_or(0) != 0,
                    skip_deleted: r.get::<_, Option<i64>>(5)?.unwrap_or(0) != 0,
                    version: r.get::<_, Option<String>>(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
    }

    let tag_names: HashMap<i64, String> = if has_table(&c, "tag")? {
        let mut st = c.prepare("SELECT id, name FROM tag")?;
        st.query_map([], |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default())))?
            .collect::<rusqlite::Result<_>>()?
    } else {
        HashMap::new()
    };

    let book_cols = "b.id_lib, b.id_inlib, coalesce(b.archive,''), coalesce(b.file,''), coalesce(b.format,'')";
    let key_of = |r: &rusqlite::Row| -> rusqlite::Result<(i64, String)> {
        let lib: i64 = r.get::<_, Option<i64>>(0)?.unwrap_or(0);
        let key = qt_book_key(r.get(1)?, &r.get::<_, String>(2)?, &r.get::<_, String>(3)?, &r.get::<_, String>(4)?);
        Ok((lib, key))
    };

    // Book tags: `book_tag` (schema ≥ 7) or the older single `book.id_tag` / `book.favorite` column.
    let mut by_tag: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
    if has_table(&c, "book")? {
        let sql = if has_table(&c, "book_tag")? {
            Some(format!("SELECT {book_cols}, t.id_tag FROM book_tag t JOIN book b ON b.id = t.id_book"))
        } else if has_column(&c, "book", "id_tag")? {
            Some(format!("SELECT {book_cols}, b.id_tag FROM book b WHERE b.id_tag > 0"))
        } else if has_column(&c, "book", "favorite")? {
            Some(format!("SELECT {book_cols}, b.favorite FROM book b WHERE b.favorite > 0"))
        } else {
            None
        };
        if let Some(sql) = sql {
            let mut st = c.prepare(&sql)?;
            let mut q = st.query([])?;
            while let Some(r) = q.next()? {
                let (lib, key) = key_of(r)?;
                by_tag.entry(r.get(5)?).or_default().push((lib, key));
            }
        }
        if has_column(&c, "book", "star")? {
            let mut st = c.prepare(&format!("SELECT {book_cols}, b.star FROM book b WHERE b.star > 0"))?;
            let mut q = st.query([])?;
            while let Some(r) = q.next()? {
                let (lib, key) = key_of(r)?;
                m.ratings.push((lib, key, r.get::<_, i64>(5)?.clamp(1, 5)));
            }
        }
    }
    let mut tag_ids: Vec<i64> = by_tag.keys().copied().collect();
    tag_ids.sort();
    for t in tag_ids {
        let mut books = by_tag.remove(&t).unwrap_or_default();
        books.sort();
        books.dedup();
        m.shelves.push(QtShelf {
            tag_id: t,
            name: tag_names.get(&t).cloned().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Tag {t}")),
            color: PALETTE[(t.unsigned_abs() as usize) % PALETTE.len()].to_string(),
            books,
        });
    }
    for (table, counter) in [("author_tag", &mut m.skipped_author_tags), ("seria_tag", &mut m.skipped_series_tags)] {
        if has_table(&c, table)? {
            *counter = c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get::<_, i64>(0))? as u64;
        }
    }
    Ok(m)
}

/// Result of [`QtMigration::apply`].
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyStats {
    /// Qt library id → `app.db` library id (existing libraries with the same path+inpx are reused).
    pub library_ids: Vec<(i64, i64)>,
    pub libraries_created: u64,
    pub shelves_created: u64,
    pub shelf_books: u64,
    pub ratings: u64,
}

impl QtMigration {
    /// Write the migration into `app.db` (already migrated to the current schema) for `user_id`,
    /// in one transaction. Idempotent: libraries are matched by `(path, inpx)`, shelves by name,
    /// existing shelf entries and ratings are kept. Books of unknown Qt libraries are skipped.
    /// Library paths are copied verbatim; the server should validate them against
    /// `FREELIB_BOOKS_DIR` and fix them up before importing.
    pub fn apply(&self, app: &mut Connection, user_id: i64) -> Result<ApplyStats, ImportError> {
        let now = freelib_catalog::util::now_rfc3339();
        let tx = app.transaction()?;
        let mut stats = ApplyStats::default();
        let mut lib_map: HashMap<i64, i64> = HashMap::new();
        for l in &self.libraries {
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM library WHERE path=?1 AND coalesce(inpx,'')=coalesce(?2,'')",
                    params![l.path, l.inpx],
                    |r| r.get(0),
                )
                .optional()?;
            let id = match existing {
                Some(id) => id,
                None => {
                    tx.execute(
                        "INSERT INTO library(name, path, inpx, first_author_only, skip_deleted) VALUES (?1,?2,?3,?4,?5)",
                        params![l.name, l.path, l.inpx, l.first_author_only, l.skip_deleted],
                    )?;
                    stats.libraries_created += 1;
                    tx.last_insert_rowid()
                }
            };
            lib_map.insert(l.qt_id, id);
            stats.library_ids.push((l.qt_id, id));
        }
        for s in &self.shelves {
            let existing: Option<i64> = tx
                .query_row("SELECT id FROM shelf WHERE user_id=?1 AND name=?2", params![user_id, s.name], |r| r.get(0))
                .optional()?;
            let shelf_id = match existing {
                Some(id) => id,
                None => {
                    tx.execute("INSERT INTO shelf(user_id, name, color) VALUES (?1,?2,?3)", params![user_id, s.name, s.color])?;
                    stats.shelves_created += 1;
                    tx.last_insert_rowid()
                }
            };
            let mut st = tx.prepare(
                "INSERT OR IGNORE INTO shelf_book(shelf_id, library_id, book_key, added_at) VALUES (?1,?2,?3,?4)",
            )?;
            for (qt_lib, key) in &s.books {
                if let Some(lib) = lib_map.get(qt_lib) {
                    stats.shelf_books += st.execute(params![shelf_id, lib, key, now])? as u64;
                }
            }
        }
        {
            let mut st = tx.prepare(
                "INSERT OR IGNORE INTO rating(user_id, library_id, book_key, rating) VALUES (?1,?2,?3,?4)",
            )?;
            for (qt_lib, key, rating) in &self.ratings {
                if let Some(lib) = lib_map.get(qt_lib) {
                    stats.ratings += st.execute(params![user_id, lib, key, rating])? as u64;
                }
            }
        }
        tx.commit()?;
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(qt_book_key(Some(123), "x.inp", "123", "fb2"), "lib:123");
        assert_eq!(qt_book_key(Some(0), "fb2-01.inp", "77", "fb2"), "file:fb2-01.zip/77.fb2");
        assert_eq!(qt_book_key(None, "sub\\a.zip", "77", "FB2"), "file:sub/a.zip/77.fb2");
        assert_eq!(qt_book_key(None, "", "dir/book", "epub"), "file:dir/book.epub");
    }

    #[test]
    fn reads_and_applies_qt_db() {
        let dir = tempfile::tempdir().unwrap();
        let qt = dir.path().join("freeLib.sqlite");
        {
            let c = Connection::open(&qt).unwrap();
            c.execute_batch(
                r#"
CREATE TABLE lib (id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT, name TEXT, path TEXT, inpx TEXT, version TEXT, firstAuthor BOOL, woDeleted BOOL);
CREATE TABLE tag (id INTEGER NOT NULL, name TEXT, id_icon INTEGER, PRIMARY KEY(id));
CREATE TABLE book (id INTEGER, name TEXT, star INTEGER, id_seria INTEGER, num_in_seria INTEGER, language TEXT, id_lib INTEGER, file TEXT, size INTEGER, deleted BOOL, date DATETIME, format TEXT, keys TEXT, id_inlib INTEGER, archive TEXT, first_author_id INTEGER, PRIMARY KEY (id));
CREATE TABLE book_tag (id_book INTEGER NOT NULL, id_tag INTEGER NOT NULL, UNIQUE (id_book, id_tag) ON CONFLICT IGNORE);
CREATE TABLE author_tag (id_author INTEGER NOT NULL, id_tag INTEGER NOT NULL);
CREATE TABLE seria_tag (id_seria INTEGER NOT NULL, id_tag INTEGER NOT NULL);
INSERT INTO lib VALUES (1, 'Флибуста', '/books/flibusta', '/books/flibusta/f.inpx', '20240101', 1, 0);
INSERT INTO lib VALUES (2, 'Folder', '/books/local', '', NULL, 0, 1);
INSERT INTO tag VALUES (1,'Favorite',1),(4,'Reading',4),(5,'To read',5),(6,'Read',6);
INSERT INTO book (id, name, star, id_lib, file, format, id_inlib, archive) VALUES
  (10, 'A', 5, 1, '100', 'fb2', 100, 'fb2-000001.inp'),
  (11, 'B', 0, 1, '101', 'fb2', 0, 'fb2-000001.inp'),
  (12, 'C', 3, 2, 'x/y', 'epub', NULL, '');
INSERT INTO book_tag VALUES (10, 1), (11, 1), (12, 6);
INSERT INTO author_tag VALUES (1, 1);
"#,
            )
            .unwrap();
        }
        let m = read_qt_database(&qt).unwrap();
        assert_eq!(m.libraries.len(), 2);
        assert!(m.libraries[0].first_author_only && !m.libraries[0].skip_deleted);
        assert_eq!(m.libraries[1].inpx, None);
        assert_eq!(m.shelves.len(), 2);
        assert_eq!(m.shelves[0].name, "Favorite");
        assert_eq!(m.shelves[0].books, vec![(1, "file:fb2-000001.zip/101.fb2".to_string()), (1, "lib:100".to_string())]);
        assert_eq!(m.shelves[1].books, vec![(2, "file:x/y.epub".to_string())]);
        assert_eq!(m.ratings.len(), 2);
        assert_eq!(m.skipped_author_tags, 1);
        assert_eq!(m.skipped_series_tags, 0);

        let mut app = freelib_catalog::open_app_db(&dir.path().join("app.db")).unwrap();
        app.execute("INSERT INTO user(id, username, password_hash, role, created_at) VALUES (1,'admin','x','admin','now')", [])
            .unwrap();
        let s = m.apply(&mut app, 1).unwrap();
        assert_eq!(s.libraries_created, 2);
        assert_eq!(s.shelves_created, 2);
        assert_eq!(s.shelf_books, 3);
        assert_eq!(s.ratings, 2);
        // idempotent
        let s2 = m.apply(&mut app, 1).unwrap();
        assert_eq!((s2.libraries_created, s2.shelves_created, s2.shelf_books, s2.ratings), (0, 0, 0, 0));
        assert_eq!(s2.library_ids, s.library_ids);
    }

    #[test]
    fn reads_shipped_empty_db() {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../freeLib/src/freeLib.sqlite");
        if p.exists() {
            let m = read_qt_database(&p).unwrap();
            assert!(m.libraries.is_empty() && m.shelves.is_empty());
        }
    }
}
