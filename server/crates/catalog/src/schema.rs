//! DDL for `lib_<id>.db` (catalog) and `app.db` (application state).

use rusqlite::Connection;

/// Bumped whenever the catalog layout changes; [`crate::Catalog::open`] refuses other versions
/// (the server then triggers a re-import).
///
/// 2: sort keys fold accented Latin letters (`Čapek` sorts and indexes under `C`).
/// 3: stems and Latin keys in the FTS tables, `vocab` (typo tolerance), `book.work_id`
///    (editions of one work).
pub const CATALOG_SCHEMA_VERSION: i64 = 3;

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
  arch_csize INTEGER, arch_method INTEGER,
  work_id INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE book_author (book_id INTEGER NOT NULL, author_id INTEGER NOT NULL, pos INTEGER NOT NULL, PRIMARY KEY (author_id, book_id)) WITHOUT ROWID;
CREATE TABLE book_genre  (book_id INTEGER NOT NULL, genre_id INTEGER NOT NULL, PRIMARY KEY (genre_id, book_id)) WITHOUT ROWID;
CREATE TABLE genre_count (genre_id INTEGER PRIMARY KEY, count INTEGER NOT NULL);
CREATE TABLE lang_count (lang TEXT PRIMARY KEY, count INTEGER NOT NULL) WITHOUT ROWID;
CREATE TABLE letter_index (kind TEXT NOT NULL, letter TEXT NOT NULL, count INTEGER NOT NULL, first_pos INTEGER NOT NULL, PRIMARY KEY (kind, letter)) WITHOUT ROWID;
CREATE VIRTUAL TABLE book_fts USING fts5(title, authors, series, keywords, stems, latin, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2', prefix='2 3');
CREATE VIRTUAL TABLE author_fts USING fts5(name, stems, latin, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE series_fts USING fts5(name, stems, latin, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
CREATE TABLE vocab (word TEXT PRIMARY KEY, freq INTEGER NOT NULL) WITHOUT ROWID;
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
CREATE INDEX book_work ON book(work_id);
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
    // v2: case-insensitive unique user names (older duplicates get their id appended),
    // user-id high-water mark (ids are never reused), persisted previous visit, mail counters
    r#"
UPDATE user SET username = username || ' (' || id || ')'
 WHERE EXISTS (SELECT 1 FROM user u2 WHERE u2.username = user.username COLLATE NOCASE AND u2.id < user.id);
CREATE UNIQUE INDEX user_username_nocase ON user(username COLLATE NOCASE);
INSERT OR REPLACE INTO setting(key, value) SELECT 'last_user_id', CAST(coalesce(max(id), 0) AS TEXT) FROM user;
ALTER TABLE user_state ADD COLUMN prev_visit TEXT;
CREATE TABLE mail_count (user_id INTEGER NOT NULL, day TEXT NOT NULL, count INTEGER NOT NULL, PRIMARY KEY (user_id, day)) WITHOUT ROWID;
"#,
    // v3: single sign-on identities (OpenID Connect issuer + subject → user), at most one per
    // user and issuer
    r#"
CREATE TABLE user_identity (issuer TEXT NOT NULL, subject TEXT NOT NULL, user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE, email TEXT, created_at TEXT NOT NULL, last_login TEXT, PRIMARY KEY (issuer, subject)) WITHOUT ROWID;
CREATE UNIQUE INDEX user_identity_user ON user_identity(user_id, issuer);
"#,
    // v4: personal API tokens (MCP), their audit log, and a per-user history of sends,
    // downloads and reads (reading profile, "already sent")
    r#"
CREATE TABLE api_token (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE, prefix TEXT NOT NULL, scopes TEXT NOT NULL, created_at TEXT NOT NULL, last_used_at TEXT, expires_at TEXT);
CREATE INDEX api_token_user ON api_token(user_id);
CREATE TABLE api_audit (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, token_id INTEGER, tool TEXT NOT NULL, ok INTEGER NOT NULL, detail TEXT NOT NULL DEFAULT '', at TEXT NOT NULL);
CREATE INDEX api_audit_user ON api_audit(user_id, id);
CREATE TABLE book_history (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, library_id INTEGER NOT NULL, book_key TEXT NOT NULL, action TEXT NOT NULL CHECK (action IN ('send','download','read')), device TEXT, at TEXT NOT NULL);
CREATE INDEX book_history_user ON book_history(user_id, library_id, at);
CREATE INDEX book_history_key ON book_history(user_id, library_id, book_key);
"#,
    // v5: each user's order of the devices they see (own and shared); the first one is their
    // default device
    r#"
CREATE TABLE device_order (user_id INTEGER NOT NULL, device_id INTEGER NOT NULL, pos INTEGER NOT NULL, PRIMARY KEY (user_id, device_id)) WITHOUT ROWID;
"#,
    // v6: OAuth for the MCP endpoint: dynamically registered clients, grants (one per
    // authorization = one "authorized app"), access and refresh tokens (SHA-256 of the secret
    // only; rotated refresh tokens stay until they expire, for reuse detection), and the grant
    // an audit entry was made with
    r#"
CREATE TABLE oauth_client (client_id TEXT PRIMARY KEY, name TEXT NOT NULL, redirect_uris TEXT NOT NULL, client_uri TEXT, created_at INTEGER NOT NULL, last_used_at INTEGER) WITHOUT ROWID;
CREATE TABLE oauth_grant (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, client_id TEXT NOT NULL, client_name TEXT NOT NULL, client_kind TEXT NOT NULL, redirect_uri TEXT NOT NULL, scopes TEXT NOT NULL, resource TEXT NOT NULL, created_at TEXT NOT NULL, last_used_at TEXT);
CREATE INDEX oauth_grant_user ON oauth_grant(user_id);
CREATE INDEX oauth_grant_client ON oauth_grant(client_id);
CREATE TABLE oauth_token (token_hash TEXT PRIMARY KEY, grant_id INTEGER NOT NULL REFERENCES oauth_grant(id) ON DELETE CASCADE, kind TEXT NOT NULL CHECK (kind IN ('access','refresh')), scopes TEXT NOT NULL, expires_at INTEGER NOT NULL, rotated_at INTEGER) WITHOUT ROWID;
CREATE INDEX oauth_token_grant ON oauth_token(grant_id);
ALTER TABLE api_audit ADD COLUMN grant_id INTEGER;
"#,
    // v7: followed authors / series and series dismissed from "Continue series" (start page),
    // keyed by the normalized name (author and series ids change between imports)
    r#"
CREATE TABLE follow (user_id INTEGER NOT NULL, library_id INTEGER NOT NULL, kind TEXT NOT NULL CHECK (kind IN ('author','series')), key TEXT NOT NULL, name TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY (user_id, library_id, kind, key)) WITHOUT ROWID;
CREATE TABLE series_dismiss (user_id INTEGER NOT NULL, library_id INTEGER NOT NULL, key TEXT NOT NULL, name TEXT NOT NULL, at TEXT NOT NULL, PRIMARY KEY (user_id, library_id, key)) WITHOUT ROWID;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_dedupes_user_names() {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(APP_MIGRATIONS[0]).unwrap();
        c.pragma_update(None, "user_version", 1).unwrap();
        c.execute_batch(
            "INSERT INTO user VALUES (1,'Bob','h','admin','t'),(2,'bob','h','reader','t'),(5,'eve','h','reader','t');",
        )
        .unwrap();
        assert_eq!(migrate_app_db(&c).unwrap(), APP_SCHEMA_VERSION);
        let names: Vec<String> = c
            .prepare("SELECT username FROM user ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(names, ["Bob", "bob (2)", "eve"]);
        assert!(
            c.execute(
                "INSERT INTO user(username, password_hash, role, created_at) VALUES ('EVE','h','reader','t')",
                []
            )
            .is_err()
        );
        let hwm: String = c
            .query_row(
                "SELECT value FROM setting WHERE key='last_user_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hwm, "5");
    }
}
