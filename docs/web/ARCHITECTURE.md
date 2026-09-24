# freeLib Web — architecture

The web edition of freeLib: a Rust server with a Svelte single-page app, shipped
as a Docker image. The Qt desktop app in `freeLib/` stays as is; the web
edition lives in `server/`, `web/`, `docker/`.

```
Browser (Svelte 5 SPA) ──HTTP/JSON + SSE──▶ freelib-server (Rust, axum)
                                             ├─ app.db             users, sessions, libraries, devices, shelves, ratings, settings
                                             ├─ lib_<id>.db        one read-only catalog per library (SQLite, WAL, FTS5)
                                             ├─ import worker      INPX → lib_<id>.new.db → atomic rename
                                             ├─ job queue          send / export / convert (N workers = CPU cores)
                                             ├─ cache/             annotations, covers (WebP), converted books
                                             └─ OPDS 1.2 feeds     /opds/…
```

## Repository layout

| Path | What |
|---|---|
| `server/Cargo.toml` | Cargo workspace |
| `server/crates/catalog` | Catalog schema, normalisation (`sort_key`), queries, genres table, `app.db` schema |
| `server/crates/import` | INPX reader, catalog builder, migration from the Qt `freeLib.sqlite`, synthetic INPX generator (`gen-inpx` bin) and benchmarks |
| `server/crates/fb2conv` | FB2 parsing (metadata, annotation, cover) and FB2 → EPUB 3 conversion (port of `freeLib/src/fb2mobi`) |
| `server/crates/server` | `freelib-server` binary: HTTP API, OPDS, auth, jobs, SMTP, static web app |
| `web/` | Svelte 5 + TypeScript + Vite SPA |
| `docker/` | Dockerfile (targets `slim` and `full`), `docker-compose.yml` |
| `docs/web/API.md` | HTTP API contract (source of truth for server and web) |

## Runtime configuration (environment)

| Variable | Default | Meaning |
|---|---|---|
| `FREELIB_PORT` | `8080` | HTTP port |
| `FREELIB_DATA_DIR` | `/data` (dev: `./data`) | `app.db`, `lib_<id>.db` |
| `FREELIB_BOOKS_DIR` | `/books` | Root under which library folders and INPX files must live (folder picker is limited to it) |
| `FREELIB_CACHE_DIR` | `/cache` (dev: `./cache`) | Previews and converted books; safe to delete |
| `FREELIB_EXPORT_DIR` | `/export` | Target of the "Server folder" device |
| `FREELIB_ADMIN_USER` / `FREELIB_ADMIN_PASSWORD` | `admin` / unset | Creates/updates the admin on start. If no users exist and no password is set, the server runs in **open mode** (no login, everyone is admin) and logs a warning |
| `FREELIB_AUTOIMPORT` | unset | Comma-separated INPX paths; on start, a library is created for each one not yet known and imported |
| `FREELIB_CALIBRE` | `ebook-convert` if on PATH | Calibre converter used for AZW3 / MOBI / PDF / other formats |
| `FREELIB_WEB_DIR` | unset | Serve the SPA from this folder instead of the embedded copy (development) |
| `FREELIB_WORKERS` | CPU cores | Conversion worker count |
| `RUST_LOG` | `info` | Logging |

## Catalog database (`lib_<id>.db`)

Written only by the importer, opened read-only (`query_only=1`, `mmap_size`) by the server.
Pragmas at runtime: `journal_mode=WAL`, `cache_size=-65536`, `temp_store=MEMORY`, `mmap_size=1073741824`.

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);          -- schema_version, inpx_version, imported_at (RFC3339), catalog_version (int, changes every import), source_inpx, first_author_only, skip_deleted
CREATE TABLE author (
  id INTEGER PRIMARY KEY,
  last TEXT NOT NULL, first TEXT NOT NULL, middle TEXT NOT NULL,
  name TEXT NOT NULL,          -- display: "Last First Middle" trimmed, single spaces
  sort_key TEXT NOT NULL,      -- normalize(name)
  book_count INTEGER NOT NULL
);
CREATE TABLE series (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort_key TEXT NOT NULL, book_count INTEGER NOT NULL);
CREATE TABLE book (
  id INTEGER PRIMARY KEY,
  book_key TEXT NOT NULL UNIQUE, -- stable across re-imports: "lib:<LIBID>" if LIBID present, else "file:<archive>/<file>.<ext>"
  title TEXT NOT NULL, sort_key TEXT NOT NULL,
  series_id INTEGER, serno INTEGER,
  first_author_id INTEGER NOT NULL,
  lang TEXT NOT NULL,          -- lower-case, e.g. "ru"
  ext TEXT NOT NULL,           -- "fb2", "epub", …
  file TEXT NOT NULL,          -- file name without extension, as in INPX
  archive TEXT NOT NULL,       -- zip name relative to library folder ("" when the book is a plain file)
  folder TEXT NOT NULL DEFAULT '',
  size INTEGER NOT NULL,
  date TEXT NOT NULL,          -- "YYYY-MM-DD" (INPX DATE)
  deleted INTEGER NOT NULL,
  lib_id INTEGER,              -- LIBID from INPX
  stars INTEGER NOT NULL DEFAULT 0,
  keywords TEXT NOT NULL DEFAULT '',
  arch_offset INTEGER,         -- byte offset of the local file header inside the zip (NULL until resolved)
  arch_csize INTEGER, arch_method INTEGER
);
CREATE TABLE book_author (book_id INTEGER NOT NULL, author_id INTEGER NOT NULL, PRIMARY KEY (author_id, book_id)) WITHOUT ROWID;
CREATE TABLE book_genre  (book_id INTEGER NOT NULL, genre_id INTEGER NOT NULL, PRIMARY KEY (genre_id, book_id)) WITHOUT ROWID;
CREATE TABLE genre_count (genre_id INTEGER PRIMARY KEY, count INTEGER NOT NULL);
CREATE TABLE letter_index (kind TEXT NOT NULL, letter TEXT NOT NULL, count INTEGER NOT NULL, first_pos INTEGER NOT NULL, PRIMARY KEY (kind, letter));
CREATE VIRTUAL TABLE book_fts USING fts5(title, authors, series, keywords, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
-- indexes (created after bulk load)
CREATE INDEX author_sort ON author(sort_key);
CREATE INDEX series_sort ON series(sort_key);
CREATE INDEX book_series ON book(series_id, serno);
CREATE INDEX book_first_author ON book(first_author_id);
CREATE INDEX book_date ON book(date);
CREATE INDEX book_lang ON book(lang);
CREATE INDEX book_ba_rev ON book_author(book_id);
CREATE INDEX book_bg_rev ON book_genre(book_id);
```

`normalize(s)`: Unicode lower-case, `ё→е`, `й` kept, strip punctuation/quotes (`«»"'.,:;!?()[]`),
collapse whitespace, trim. The first character of `sort_key` (upper-cased, `Ё→Е`) is the letter for
`letter_index`; non-letters go to `#`. Authors/series list order = `sort_key, id`.

Genres are global and static (`crates/catalog/data/genres.json`, exported from the Qt app's
`genre` table: `id, name, parent, keys[]` where `keys` are FB2 genre codes). Unknown codes map to
the genre named "Прочее" under the matching top-level group when possible, else to top-level
"Прочее" (id 11).

## Application database (`app.db`)

```sql
CREATE TABLE user (id INTEGER PRIMARY KEY, username TEXT UNIQUE NOT NULL, password_hash TEXT NOT NULL, role TEXT NOT NULL CHECK (role IN ('admin','reader')), created_at TEXT NOT NULL);
CREATE TABLE session (token TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE, expires_at TEXT NOT NULL);
CREATE TABLE library (id INTEGER PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL, inpx TEXT, first_author_only INTEGER NOT NULL DEFAULT 0, skip_deleted INTEGER NOT NULL DEFAULT 0, is_default INTEGER NOT NULL DEFAULT 0, auto_check TEXT);
CREATE TABLE device (id INTEGER PRIMARY KEY, user_id INTEGER, name TEXT NOT NULL, kind TEXT NOT NULL, format TEXT NOT NULL, target TEXT, file_name TEXT NOT NULL, options TEXT NOT NULL); -- options = JSON (ConvertOptions); user_id NULL = shared
CREATE TABLE shelf (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, name TEXT NOT NULL, color TEXT NOT NULL);
CREATE TABLE shelf_book (shelf_id INTEGER NOT NULL REFERENCES shelf(id) ON DELETE CASCADE, library_id INTEGER NOT NULL, book_key TEXT NOT NULL, added_at TEXT NOT NULL, PRIMARY KEY (shelf_id, library_id, book_key));
CREATE TABLE rating (user_id INTEGER NOT NULL, library_id INTEGER NOT NULL, book_key TEXT NOT NULL, rating INTEGER NOT NULL, PRIMARY KEY (user_id, library_id, book_key));
CREATE TABLE user_state (user_id INTEGER PRIMARY KEY, last_visit TEXT, prefs TEXT NOT NULL DEFAULT '{}');
CREATE TABLE setting (key TEXT PRIMARY KEY, value TEXT NOT NULL); -- JSON values: smtp, opds, …
```

User data is keyed by `(library_id, book_key)`, so it survives re-imports.

## Import (swap-in)

1. Create `lib_<id>.new.db` with `journal_mode=OFF, synchronous=OFF`, tables without indexes.
2. Read the INPX zip (`structure.info` optional, default field order
   `AUTHOR;GENRE;TITLE;SERIES;SERNO;FILE;SIZE;LIBID;DEL;EXT;DATE;LANG;STARS;KEYWORDS;`, fields separated by `\x04`,
   authors `Last,First,Middle:` list, genres `code:` list). Parse `.inp` parts in parallel, one writer.
   The `.inp` name (minus extension) + ".zip" is the archive name, unless a FOLDER field is present.
3. Build indexes, counts, `letter_index`, FTS; `ANALYZE`; write `meta`.
4. `rename(new, lib_<id>.db)`; the server reopens the pool. Readers of the old file finish undisturbed.

Progress is reported through a callback `(parts_done, parts_total, message)`; the server turns it into job events.

## Previews and conversions

- Annotation + cover are extracted by `fb2conv::read_info` on first view and cached as
  `cache/info/<lib>/<book_key-hash>.json` and `cache/covers/<lib>/<hash>-{thumb,full}.webp`.
- Book bytes: seek to `arch_offset` if known, else open the zip (LRU of open archives).
- Converted outputs cached at `cache/out/<lib>/<hash>-<profilehash>.<ext>`.
- EPUB 3 from `fb2conv`. AZW3/MOBI/PDF via Calibre (`FREELIB_CALIBRE`) from that EPUB; 501 when Calibre is absent.
  KEPUB = `fb2conv` EPUB with Kobo spans (`fb2conv::to_kepub`).

## Performance targets (full Flibusta-size INPX, ~600k books, 4 cores)

| Operation | Target |
|---|---|
| Full import | < 3 min |
| `GET authors` list (cold, compressed) | < 300 ms server |
| Books of an author / series | < 50 ms p95 |
| Search | < 300 ms p95 |
| Preview after first view | < 20 ms |
