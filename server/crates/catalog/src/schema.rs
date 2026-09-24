//! DDL for `lib_<id>.db` (catalog) and `app.db` (application state).

use rusqlite::Connection;

/// Bumped whenever the catalog layout changes; [`crate::Catalog::open`] refuses other versions
/// (the server then triggers a re-import).
pub const CATALOG_SCHEMA_VERSION: i64 = 1;

/// Catalog tables. Created on an empty database by the importer *before* the bulk load;
/// indexes ([`CATALOG_INDEXES`]) are created afterwards.
pub const CATALOG_TABLES: &str = r#"
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT) WITHOUT ROWID;
CREATE TABLE author (
  id INTEGER PRIMARY KEY,
  last TEXT NOT NULL, first TEXT NOT NULL, middle TEXT NOT NULL,
  name TEXT NOT NULL,
  sort_key TEXT NOT NULL,
  book_count INTEGER NOT NULL
);
CREATE TABLE series (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  sort_key TEXT NOT NULL,
  book_count INTEGER NOT NULL,
  authors TEXT NOT NULL DEFAULT ''
);
CREATE TABLE book (
  id INTEGER PRIMARY KEY,
  book_key TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL, sort_key TEXT NOT NULL,
  series_id INTEGER, serno INTEGER,
  first_author_id INTEGER NOT NULL,
  lang TEXT NOT NULL,
  ext TEXT NOT NULL,
  file TEXT NOT NULL,
  archive TEXT NOT NULL,
  folder TEXT NOT NULL DEFAULT '',
  size INTEGER NOT NULL,
  date TEXT NOT NULL,
  deleted INTEGER NOT NULL,
  lib_id INTEGER,
  stars INTEGER NOT NULL DEFAULT 0,
  keywords TEXT NOT NULL DEFAULT '',
  arch_offset INTEGER,
  arch_csize INTEGER, arch_method INTEGER
);
CREATE TABLE book_author (book_id INTEGER NOT NULL, author_id INTEGER NOT NULL, pos INTEGER NOT NULL, PRIMARY KEY (author_id, book_id)) WITHOUT ROWID;
CREATE TABLE book_genre  (book_id INTEGER NOT NULL, genre_id INTEGER NOT NULL, PRIMARY KEY (genre_id, book_id)) WITHOUT ROWID;
CREATE TABLE genre_count (genre_id INTEGER PRIMARY KEY, count INTEGER NOT NULL);
CREATE TABLE lang_count (lang TEXT PRIMARY KEY, count INTEGER NOT NULL) WITHOUT ROWID;
CREATE TABLE letter_index (kind TEXT NOT NULL, letter TEXT NOT NULL, count INTEGER NOT NULL, first_pos INTEGER NOT NULL, PRIMARY KEY (kind, letter)) WITHOUT ROWID;
CREATE VIRTUAL TABLE book_fts USING fts5(title, authors, series, keywords, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2', prefix='2 3');
CREATE VIRTUAL TABLE author_fts USING fts5(name, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE series_fts USING fts5(name, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
"#;

/// Indexes built after the bulk load.
pub const CATALOG_INDEXES: &str = r#"
CREATE INDEX author_sort ON author(sort_key);
CREATE INDEX series_sort ON series(sort_key);
CREATE INDEX book_series ON book(series_id, serno);
CREATE INDEX book_first_author ON book(first_author_id);
CREATE INDEX book_date ON book(date);
CREATE INDEX book_lang ON book(lang);
CREATE INDEX book_ba_rev ON book_author(book_id, pos);
CREATE INDEX book_bg_rev ON book_genre(book_id);
"#;

/// Create all catalog tables on an empty connection.
pub fn create_catalog_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(CATALOG_TABLES)
}

/// Create catalog indexes (after bulk load).
pub fn create_catalog_indexes(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(CATALOG_INDEXES)
}

/// `app.db` migrations. Entry `i` upgrades `user_version` from `i` to `i + 1`.
/// Never edit a shipped entry; append a new one.
pub const APP_MIGRATIONS: &[&str] = &[
    // v1: initial schema (docs/web/ARCHITECTURE.md)
    r#"
CREATE TABLE user (id INTEGER PRIMARY KEY, username TEXT UNIQUE NOT NULL, password_hash TEXT NOT NULL, role TEXT NOT NULL CHECK (role IN ('admin','reader')), created_at TEXT NOT NULL);
CREATE TABLE session (token TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE, expires_at TEXT NOT NULL);
CREATE TABLE library (id INTEGER PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL, inpx TEXT, first_author_only INTEGER NOT NULL DEFAULT 0, skip_deleted INTEGER NOT NULL DEFAULT 0, is_default INTEGER NOT NULL DEFAULT 0, auto_check TEXT);
CREATE TABLE device (id INTEGER PRIMARY KEY, user_id INTEGER, name TEXT NOT NULL, kind TEXT NOT NULL, format TEXT NOT NULL, target TEXT, file_name TEXT NOT NULL, options TEXT NOT NULL);
CREATE TABLE shelf (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, name TEXT NOT NULL, color TEXT NOT NULL);
CREATE TABLE shelf_book (shelf_id INTEGER NOT NULL REFERENCES shelf(id) ON DELETE CASCADE, library_id INTEGER NOT NULL, book_key TEXT NOT NULL, added_at TEXT NOT NULL, PRIMARY KEY (shelf_id, library_id, book_key));
CREATE TABLE rating (user_id INTEGER NOT NULL, library_id INTEGER NOT NULL, book_key TEXT NOT NULL, rating INTEGER NOT NULL, PRIMARY KEY (user_id, library_id, book_key));
CREATE TABLE user_state (user_id INTEGER PRIMARY KEY, last_visit TEXT, prefs TEXT NOT NULL DEFAULT '{}');
CREATE TABLE setting (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE INDEX session_user ON session(user_id);
CREATE INDEX shelf_user ON shelf(user_id);
CREATE INDEX device_user ON device(user_id);
"#,
];

/// Current `app.db` schema version (`PRAGMA user_version` after migrating).
pub const APP_SCHEMA_VERSION: i64 = APP_MIGRATIONS.len() as i64;

/// Open (creating if needed) and migrate `app.db`; sets WAL, foreign keys and a busy timeout.
pub fn open_app_db(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate_app_db(&conn)?;
    Ok(conn)
}

/// Apply pending `app.db` migrations inside one transaction each.
/// Returns the resulting schema version. Fails if the database is newer than this binary.
pub fn migrate_app_db(conn: &Connection) -> rusqlite::Result<i64> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if current > APP_SCHEMA_VERSION {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "app.db schema version {current} is newer than supported {APP_SCHEMA_VERSION}"
        )));
    }
    for (i, sql) in APP_MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(APP_SCHEMA_VERSION)
}
