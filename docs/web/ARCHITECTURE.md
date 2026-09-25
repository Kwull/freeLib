# freeLib Web — architecture

The web edition of freeLib: a Rust server with a Svelte single-page app, shipped
as a Docker image. The Qt desktop app in `freeLib/` stays as is; the web
edition lives in `server/`, `web/`, `docker/`.

```
Browser (Svelte 5 SPA) ──HTTP/JSON + SSE──▶ freelib-server (Rust, axum)
                                             ├─ app.db             users, sessions, libraries, devices, shelves, ratings, settings
                                             ├─ lib_<id>.db        one read-only catalog per library (SQLite, FTS5)
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
| `FREELIB_ADMIN_USER` / `FREELIB_ADMIN_PASSWORD` | `admin` / unset | Creates/updates the admin on start. If no users exist, no password is set and single sign-on is not configured, the server runs in **open mode** (no login, everyone is admin) and logs a warning |
| `FREELIB_AUTOIMPORT` | unset | Comma-separated INPX paths; on start, a library is created for each one not yet known and imported |
| `FREELIB_CALIBRE` | `ebook-convert` if on PATH | Calibre converter used for AZW3 / MOBI / PDF (EPUB input only; versions before 6.19 are refused, CVE-2023-46303) |
| `FREELIB_WEB_DIR` | unset | Serve the SPA from this folder instead of the embedded copy (development) |
| `FREELIB_WORKERS` | CPU cores | Conversion worker count |
| `FREELIB_BIND` | `0.0.0.0` | Listen address |
| `FREELIB_TRUST_PROXY` | unset | `1`: the server is only reachable through one reverse proxy; the client address for login rate limiting is `X-Real-IP`, else the rightmost `X-Forwarded-For` entry |
| `FREELIB_ALLOWED_HOSTS` | unset | Comma-separated host names (`books.example.org`, `*.lan`) accepted in the `Host` header besides `localhost` and IP literals. Checked in open mode always (DNS rebinding protection: other names get 421) and in every mode once set |
| `FREELIB_PUBLIC_URL` | unset | External base URL (`https://books.example.org`, no trailing slash). Required for single sign-on (redirect URI `<FREELIB_PUBLIC_URL>/api/v1/auth/oidc/callback`); an `https://` URL also makes every cookie `Secure` |
| `FREELIB_OIDC_ISSUER` / `FREELIB_OIDC_CLIENT_ID` | unset | Enable OpenID Connect sign-in (both required; the server refuses to start with only one, or without `FREELIB_PUBLIC_URL`). The issuer must match the provider's discovery document exactly |
| `FREELIB_OIDC_CLIENT_SECRET` | unset | For confidential clients; public clients rely on PKCE alone |
| `FREELIB_OIDC_SCOPES` | `openid profile email` | Space- or comma-separated; `openid` is always added. Add `groups` for providers that only send the claim when asked (Pocket ID, Authentik) |
| `FREELIB_OIDC_BUTTON` | `Sign in with SSO` | Label of the sign-in button |
| `FREELIB_OIDC_ADMIN_GROUP` | unset | Members of this group (`groups` claim, from the ID token or userinfo) become administrators, everybody else a reader; re-evaluated at every SSO sign-in (the `FREELIB_ADMIN_USER` account is never demoted) |
| `FREELIB_OIDC_AUTO_CREATE` | `true` | Create a reader account on the first sign-in of an unknown identity; `false`: only identities an administrator's users linked themselves can sign in |
| `FREELIB_OIDC_DISABLE_PASSWORD` | `false` | Hide and refuse password sign-in in the web app, except for the `FREELIB_ADMIN_USER` account while `FREELIB_ADMIN_PASSWORD` is set (the way back in). OPDS keeps Basic auth |
| `FREELIB_CACHE_MAX_MB` | `2048` | Size bound of `cache/{out,covers,info}`; least recently used files are evicted (`0` = unbounded) |
| `FREELIB_CALIBRE_TIMEOUT` | `300` | Seconds before a Calibre conversion is killed (`FREELIB_CALIBRE=none` disables Calibre) |
| `RUST_LOG` | `info` | Logging |

## Catalog database (`lib_<id>.db`)

Written only by the importer; the server opens it read-only (`SQLITE_OPEN_READ_ONLY`, `query_only=1`)
through a small per-library connection pool that is swapped atomically after a re-import
(`freelib_catalog::CatalogHandle`, see `server/crates/catalog/README.md`).
Runtime pragmas: `cache_size=-65536`, `temp_store=MEMORY`, `mmap_size=1073741824`, `busy_timeout=5s`.
The file is left in rollback-journal mode (`journal_mode=DELETE`), not WAL: it is never written while
served, and a read-only WAL database would need writable `-wal`/`-shm` files next to it.

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT) WITHOUT ROWID;
  -- schema_version, inpx_version, collection_name, imported_at (RFC3339), catalog_version (int, strictly increasing,
  -- ms timestamp), source_inpx, first_author_only, skip_deleted, book_count, live_book_count, author_count, series_count
CREATE TABLE author (
  id INTEGER PRIMARY KEY,
  last TEXT NOT NULL, first TEXT NOT NULL, middle TEXT NOT NULL,
  name TEXT NOT NULL,          -- display: "Last First Middle" trimmed, single spaces
  sort_key TEXT NOT NULL,      -- normalize(name)
  book_count INTEGER NOT NULL  -- live (non-deleted) books
);
CREATE TABLE series (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort_key TEXT NOT NULL, book_count INTEGER NOT NULL,
  authors TEXT NOT NULL DEFAULT '');  -- display names of the ≤ 2 most frequent first authors (search result "authors")
CREATE TABLE book (
  id INTEGER PRIMARY KEY,
  book_key TEXT NOT NULL UNIQUE, -- stable across re-imports: "lib:<LIBID>" if LIBID present, else "file:<archive or folder>/<file>.<ext>"
  title TEXT NOT NULL, sort_key TEXT NOT NULL,
  series_id INTEGER, serno INTEGER,  -- serno NULL when empty/0
  first_author_id INTEGER NOT NULL,
  lang TEXT NOT NULL,          -- lower-case, first 2 chars (Qt compatible), e.g. "ru"
  ext TEXT NOT NULL,           -- lower-case, "fb2", "epub", …
  file TEXT NOT NULL,          -- file name without extension, as in INPX
  archive TEXT NOT NULL,       -- zip name relative to library folder ("" when the book is a plain file)
  folder TEXT NOT NULL DEFAULT '', -- raw INPX FOLDER value ("" when absent); for plain files the sub-folder holding the file
  size INTEGER NOT NULL,
  date TEXT NOT NULL,          -- "YYYY-MM-DD" (INPX DATE) or "" when missing/invalid
  deleted INTEGER NOT NULL,
  lib_id INTEGER,              -- LIBID from INPX
  stars INTEGER NOT NULL DEFAULT 0,
  keywords TEXT NOT NULL DEFAULT '',
  arch_offset INTEGER,         -- byte offset of the local file header inside the zip (NULL until resolved)
  arch_csize INTEGER, arch_method INTEGER
);
CREATE TABLE book_author (book_id INTEGER NOT NULL, author_id INTEGER NOT NULL,
  pos INTEGER NOT NULL,        -- author order in the INPX record (0 = first author)
  PRIMARY KEY (author_id, book_id)) WITHOUT ROWID;
CREATE TABLE book_genre  (book_id INTEGER NOT NULL, genre_id INTEGER NOT NULL, PRIMARY KEY (genre_id, book_id)) WITHOUT ROWID;
CREATE TABLE genre_count (genre_id INTEGER PRIMARY KEY, count INTEGER NOT NULL);  -- live books; groups: distinct books in the group
CREATE TABLE lang_count (lang TEXT PRIMARY KEY, count INTEGER NOT NULL) WITHOUT ROWID;  -- live books per language
CREATE TABLE letter_index (kind TEXT NOT NULL, letter TEXT NOT NULL, count INTEGER NOT NULL, first_pos INTEGER NOT NULL, PRIMARY KEY (kind, letter)) WITHOUT ROWID;
-- FTS rows hold normalize()d text; rowid = book / author / series id.
CREATE VIRTUAL TABLE book_fts USING fts5(title, authors, series, keywords, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2', prefix='2 3');
CREATE VIRTUAL TABLE author_fts USING fts5(name, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE series_fts USING fts5(name, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
-- indexes (created after bulk load)
CREATE INDEX author_sort ON author(sort_key);
CREATE INDEX series_sort ON series(sort_key);
CREATE INDEX book_series ON book(series_id, serno);
CREATE INDEX book_first_author ON book(first_author_id);
CREATE INDEX book_date ON book(date);
CREATE INDEX book_lang ON book(lang);
CREATE INDEX book_ba_rev ON book_author(book_id, pos);
CREATE INDEX book_bg_rev ON book_genre(book_id);
```

Additions to the original design and why:

| Addition | Reason |
|---|---|
| `book_author.pos`, `book_ba_rev(book_id, pos)` | keeps the INPX author order; authors of a whole page load with one covering-index query |
| `series.authors` | search results show series authors without a per-row query |
| `lang_count`, `meta` counts | `GET /languages` and library stats without scanning `book` |
| `author_fts`, `series_fts` | author/series search matches any word of the name in any order ("аркадий стругацкий"), not only the sort-key start |
| `book_fts … prefix='2 3'` | prefix indexes for 2–3 letter prefixes: +31 MB on 600k books; search p95 ≈ 50 ms instead of 62 ms, worst query ≈ 65 ms instead of 102 ms |
| meta / letter_index / lang_count `WITHOUT ROWID` | tiny key-value tables, one B-tree lookup |
| `journal_mode=DELETE` instead of WAL | see above |

`normalize(s)`: Unicode lower-case, `ё→е`, `й` kept, quotes/apostrophes dropped (`'"«»„“”‘’` — `O'Brien` → `obrien`),
other punctuation (`.,:;!?()[]{}…—–/\|`) turned into spaces, whitespace collapsed, trimmed, leading
non-alphanumeric characters dropped (`- Hello` → `hello`). Keys compare byte-wise (digits < Latin < Cyrillic).
The first character of `sort_key` (upper-cased, `Ё→Е`) is the letter for `letter_index`; non-letters go to `#`.
Authors/series list order = `sort_key, id`.

Authors are deduplicated by `normalize(name)` and series by `normalize(name)` (so `Ёлкин Пётр` = `елкин петр`,
`Лес` = `лес`); the first spelling seen is displayed.

Genres are global and static (`crates/catalog/data/genres.json`, exported from the Qt app's
`genre` table: `id, name, parent, keys[]` where `keys` are FB2 genre codes). Codes are matched lower-case with
spaces as `_` (as Qt does). Unknown codes map to the "…: прочее" genre of the top-level group that most codes
with the same prefix (text before the first `_`) belong to — `sf_brand_new` → "Фантастика: прочее" — else to
top-level "Прочее" (id 11). A book's genre ids are deduplicated; books without genre codes have no genre.

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

Indexes added: `session(user_id)`, `shelf(user_id)`, `device(user_id)`.

Migration v2: unique index `user(username COLLATE NOCASE)` (older case-only duplicates get ` (<id>)` appended),
`setting.last_user_id` (user ids are never reused, so a deleted user's jobs or files can never match a new user),
`user_state.prev_visit` (start of the previous visit, the `newSinceLastVisit` baseline; books dated on or after
its server-local day count as new), and
`mail_count(user_id, day, count)` for `smtp.dailyLimitPerUser`.
Migration v3: `user_identity(issuer, subject, user_id, email, created_at, last_login)`, primary key
`(issuer, subject)`, unique `(user_id, issuer)`: the single sign-on identity linked to an account. Accounts
created by single sign-on have an empty `password_hash` (no password sign-in, no OPDS) until the user sets one.
Migrations: `freelib_catalog::schema::APP_MIGRATIONS` is an append-only list of SQL batches; `PRAGMA user_version`
holds how many were applied; `open_app_db()` applies the missing ones, each in a transaction, and refuses a newer database.

User data is keyed by `(library_id, book_key)`, so it survives re-imports.

## Single sign-on (OpenID Connect)

`server/crates/server/src/oidc.rs` (flow, validation, accounts) and `api/oidc.rs` (endpoints, cookies), built on
the `openidconnect` crate with `reqwest` + rustls (web PKI and system roots, `SSL_CERT_FILE` honoured).

* **Discovery** `<issuer>/.well-known/openid-configuration` and the JWKS are fetched on start (failure is only
  logged) and cached for an hour; a failed refresh keeps the cached copy. An ID token signed with a key that is not
  in the cached JWKS triggers one immediate refetch (key rotation). The HTTP client follows no redirects (SSRF) and
  times out after 20 s.
* **Start** (`GET /auth/oidc/login`): PKCE S256 verifier, random `state` and `nonce`. The pending sign-in
  (nonce, verifier, return path, user id for the link flow) is kept in server memory for 10 minutes (at most 10 000;
  lost on restart, the user simply retries); the browser gets `state` in an HttpOnly, `SameSite=Lax` cookie scoped to
  `/api/v1/auth/oidc`. The return path is reduced to a same-origin relative path (`/…`, not `//…`, no backslashes,
  control characters, `/api/`, `/opds`, `/login`), else `/`.
* **Callback**: the `state` parameter must equal the cookie (constant-time) and name a pending sign-in, which is
  removed (single use) — this binds the callback to the browser that started it (login CSRF). Then the code is
  exchanged with the PKCE verifier (client secret via `client_secret_basic`, or `client_secret_post` when the
  provider lists only that). ID token checks: signature with an allowed asymmetric algorithm (RS/PS 256–512,
  ES256/384, EdDSA; no `none`, no HMAC), `iss` equals the issuer, `aud` contains the client id (other audiences
  refused), `exp` with 60 s leeway, `iat` not more than 60 s in the future nor older than 10 minutes, `nonce`, and
  `at_hash` when present. Userinfo is fetched (and its `sub` must match) when the ID token lacks `groups` while
  `FREELIB_OIDC_ADMIN_GROUP` is set, or lacks all of `preferred_username`, `email`, `name`.
* **Accounts**: found by (issuer, `sub`) in `user_identity`, never by name or e-mail. A new identity creates a reader
  (with `FREELIB_OIDC_AUTO_CREATE`) named after `preferred_username`, else `email`, else `name`, made unique with
  ` (2)`, ` (3)`… — a provider user called `admin` becomes `admin (2)`, never the local admin. The admin group sets
  the role at every sign-in. Linking (`POST /auth/oidc/link`, a same-origin JSON request, so CSRF-protected) stores the
  user id in the pending sign-in; the callback links only when the browser is still signed in as that user.
* **Sessions**: the existing `session` table and cookie; every sign-in (password or SSO) and every link issues a new
  session id and invalidates the one the browser sent (session fixation). Failed callbacks count against the login
  rate limiter per client address. Cookies are `Secure` when `FREELIB_PUBLIC_URL` is https or the proxy sends
  `X-Forwarded-Proto: https`. Request spans log the path only (no `code`/`state`); tokens are never logged.
* With single sign-on configured the server never runs in open mode. OPDS keeps HTTP Basic with local passwords.

## Import (swap-in)

1. Create `lib_<id>.new.db` with `journal_mode=OFF, synchronous=OFF, locking_mode=EXCLUSIVE`, tables without indexes.
2. Read the INPX zip (`structure.info` optional, default field order
   `AUTHOR;GENRE;TITLE;SERIES;SERNO;FILE;SIZE;LIBID;DEL;EXT;DATE;LANG;STARS;KEYWORDS;`, fields separated by `\x04`,
   authors `Last,First,Middle:` list, genres `code:` list). Parse `.inp` parts in parallel (rayon, CPU cores − 1
   threads, one zip handle per thread), one writer thread inserting in part order (book ids are deterministic).
   The `.inp` name (minus extension) + ".zip" is the archive name, unless a FOLDER field is present.
   With a library folder, each worker also reads the central directory of the part's archive(s) once and fills
   `arch_offset/arch_csize/arch_method` (missing archives are listed in the import stats and skipped).
3. Write authors/series/counts/`letter_index` (aggregated in memory), build indexes, `optimize` the FTS tables,
   `ANALYZE`, write `meta`, switch to `journal_mode=DELETE`, fsync.
4. `rename(new, lib_<id>.db)`; the server calls `CatalogHandle::reload()`. Readers of the old file finish undisturbed.
   On error or cancellation the `.new.db` is deleted and the current catalog is untouched.

While a library has no usable catalog (first import, or a catalog of another schema version being rebuilt on start —
`status.reason: "upgrade"`), it cannot be browsed: the web app shows the import card with the live progress from
`library` events (polling `GET /libraries` when the event stream is down) and continues by itself when the import
ends. A re-import of a browsable library keeps serving the old catalog until the swap.

Progress is reported through a callback `(done, total, message)` with `total = parts + 5` (the finishing steps count
as one each); the server turns it into job events. Cancellation: an `AtomicBool` checked per part and between steps.

**Mode "new"** (only add new books) is implemented as a full rebuild: a Flibusta-size import takes ~20 s,
user data is keyed by `book_key`, and a rebuild also picks up deletions and corrections — so there is no
incremental path to maintain. The server may simply start a full import for `{mode: "new"}`.

INPX parsing details (compared with `importthread.cpp`):

* `structure.info` (any case, BOM tolerated): field names on the first non-empty line; `LIBRATE` is an alias of
  `STARS`; unknown names (`INSNO`, `URI`, `TAG…`) are ignored; missing fields read as empty.
* Lines split on `\n` (trailing `\r` removed); lines without `\x04` or without FILE are skipped; invalid UTF-8 is replaced.
* Authors: empty entries skipped, duplicates removed, `Автор неизвестен`/`неизвестно`/`unknown` variants and books
  with no author map to one author "Автор неизвестен" (dropped when real authors are present). Qt's fallback of
  reading the FB2 for unknown authors is not done (too slow for bulk import).
* `first_author_only` keeps only the first author (Qt applied it only to FB2 folder imports); `skip_deleted` drops
  records with `DEL` > 0.
* SERNO / SIZE / LIBID / STARS: integers (leading digits accepted); SERNO 0 → NULL; LIBID 0 → none; STARS clamped 0..5.
  DATE validated (`YYYY-M-D` normalised), invalid → `""`. LANG lower-case, first 2 chars. EXT lower-case, leading dot removed.
* FOLDER: `x.zip` → archive `x.zip`; `x.inp` → archive `x.zip`; any other non-empty value is a plain sub-folder
  (archive `""`, file at `<folder>/<file>.<ext>`); empty → derived from the `.inp` name. `\` becomes `/`.
* Duplicate `book_key`: a repeated LIBID is stored under its `file:` key; an exact duplicate is dropped (both counted in the stats).

`freelib_import::resolve_offsets(db, library_dir, …)` resolves offsets for an existing catalog (e.g. imported before
the archives were mounted); it updates the file in place, so run it when the catalog is not busy, then `reload()`.

### Migration from the Qt app

`freelib_import::migrate::read_qt_database(freeLib.sqlite) -> QtMigration` returns plain data:
libraries (`lib`: name, path, inpx, firstAuthor → `first_author_only`, woDeleted → `skip_deleted`, version),
shelves (one per Qt tag that has tagged books — `book_tag`, or the pre-v7 `book.id_tag`/`favorite` column — with a
palette colour) and ratings (`book.star > 0`). Books are referenced by `book_key` computed from the old row
(`lib:<id_inlib>`, else `file:<archive with .inp→.zip>/<file>.<format>`), so they attach after the libraries are
re-imported. Author and series tags have no web equivalent and are only counted. `QtMigration::apply(&mut app_db, user_id)`
writes it all in one transaction (idempotent: libraries matched by path+inpx, shelves by name, existing rows kept);
paths are copied verbatim, the server should validate them against `FREELIB_BOOKS_DIR`. Note that Qt stored INPX
`LIBRATE` values in `book.star` too, so migrated ratings include those.

## Previews and conversions

- Annotation + cover are extracted by `fb2conv::read_info` on first view and cached as
  `cache/info/<lib>/<book_key-hash>.json` and `cache/covers/<lib>/<hash>-thumb.webp` (240 px high, lossy WebP) +
  `<hash>-full.{jpg,png,webp}` (JPEG/PNG covers kept as they are, other formats re-encoded to WebP).
- Book bytes: seek to `arch_offset` if known, else open the zip (LRU of open archives).
- Converted outputs cached at `cache/out/<lib>/<hash>-<profilehash>.<ext>`. The cache is bounded by
  `FREELIB_CACHE_MAX_MB`: after writes and every 10 minutes the least recently used files (by modification
  time, refreshed on cache hits) are deleted down to 90 % of the limit; interrupted atomic writes
  (`*.tmp<hex>`) are removed at startup.
- EPUB 3 from `fb2conv`. AZW3/MOBI/PDF via Calibre (`FREELIB_CALIBRE`) from that EPUB (or an original
  `.epub`); 501 when Calibre is absent. Calibre never sees other inputs (HTML, TXT, DOCX, …: those books are
  offered as originals only), runs in a private temporary directory (cwd, `HOME`, config and temp dirs), and is
  killed when its job is cancelled. Calibre older than 6.19 is treated as missing.
- Resource limits: books are read through bounded readers (256 MiB uncompressed, `.inp` parts 512 MiB, zip
  entries never trusted for their declared size), images are decoded with dimension (8000 px) and allocation
  (128 MiB) limits, whole-book previews and conversions run under semaphores, Argon2 verifications are limited
  to two at a time.
  KEPUB = `fb2conv` EPUB with Kobo spans (`fb2conv::to_kepub`).

## Performance targets (full Flibusta-size INPX, ~600k books, 4 cores)

Measured with `cargo run --release -p freelib-import --bin bench` on a synthetic INPX
(`gen-inpx --books 600000`: 600k books, 300 parts, 113k authors, 56k series, 20 MB INPX, 267 MB catalog) on a
4-core / 15 GB VM, warm page cache. Numbers are the catalog layer only (no HTTP, no compression).

| Operation | Target | Measured |
|---|---|---|
| Full import | < 3 min | 16.7 s; 18.4 s incl. zip offsets for all 600k books (744 MB of archives) |
| `GET authors` list (cold, compressed) | < 300 ms server | 94 ms query (first call), 120 ms incl. JSON (6.7 MB for 113k rows, before compression); series list 42 ms |
| Books of an author / series | < 50 ms p95 | author p95 2.6 ms (max 14 ms, 1621 books); series p95 0.1 ms |
| Search | < 300 ms p95 | p50 15 ms, p95 45 ms, max 66 ms (25 queries incl. 2-letter prefixes, up to 71k matches, with facets) + one-time 260 ms attribute load per catalog |
| Preview after first view | < 20 ms | (server cache; `book(id)` itself: p95 0.03 ms) |

Other queries (p95): leaf genre, first page of 2000: 27 ms; top-level genre group: 67 ms; `since` 30 days / 1 year: 14 / 15 ms;
500 shelf ids: 4 ms; 1000 `book_key` lookups: 2 ms; genres with counts: 0.07 ms; languages: 0.01 ms.
Loading a 2000-book page (books + authors + genres, 3 queries) is ~15 ms of the genre timings.
The real Flibusta catalog has more authors than the synthetic one (~200k); the authors list scales linearly
(≈ 1 µs per row for the query plus JSON), so expect ~200 ms before compression — the server should cache the
serialised, compressed list per `catalogVersion`.
Reproduce: `cargo run --release -p freelib-import --bin bench` (generates `server/bench-data/synthetic-600k.inpx`
if missing). With archives and offset resolution:
`gen-inpx --books 600000 --out bench-data/files600k/lib.inpx --with-files bench-data/files600k/lib`, then
`bench --inpx bench-data/files600k/lib.inpx --lib-dir bench-data/files600k/lib --db bench-data/files_lib.db`.

### Measured: HTTP server

Release `freelib-server`, same 4-core / 15 GB VM, open mode, library `bench-data/files600k`
(`lib.inpx` + 300 zip archives with real FB2 files; 600k records, 551,652 live books, 112,812 authors,
56,347 series). Timings are `curl` wall-clock times on localhost, including HTTP.

| Operation | Target | Measured |
|---|---|---|
| Auto-import at start (`FREELIB_AUTOIMPORT`, incl. zip offsets) | < 3 min | 16.1 s, then 0.8–1.2 s background warm-up (authors/series lists + Brotli, search attributes, gzip) |
| `GET authors`, cold (first call right after a restart, list built, Brotli q5) | < 300 ms | 268 ms, 1,018,665 bytes br (6,743,259 bytes JSON) |
| `GET authors`, cached (per catalog version) | | 1.3–2.3 ms; gzip 1,034,875 bytes; `?v=` → `immutable`; `If-None-Match` → 304 |
| `GET series`, cold / cached | | 128 ms / 1.6 ms, 461,618 bytes br |
| Books of an author (200 random authors) | < 50 ms p95 | p50 1.5 ms, p95 2.2 ms, max 8 ms; the biggest author (1621 books, 50 KB br): 33–41 ms |
| Search (25 queries incl. 2-letter prefixes, with facets) | < 300 ms p95 | p50 22 ms, p95 64 ms, max 83 ms |
| Book detail (50 random books) first call / cached | < 20 ms cached | first p50 2.2 ms, p95 2.9 ms (reads the FB2, extracts annotation + cover); then p50 1.7 ms |
| Cover thumbnail (240 px WebP) first / cached | | 2.1 ms / 1.5 ms (synthetic covers are 60×90 PNG, so thumbnails are small) |
| Original file (FB2 via `arch_offset`) | | 2 ms |
| EPUB download first (conversion) / cached | | p50 36 ms, max 70 ms (a generated 336 KB cover dominates) / 3.9 ms |
| KEPUB (from the cached EPUB) | | 3 ms |
| Memory (RSS) after start + warm-up | | ≈ 250 MB; ≈ 880 MB right after the import (SQLite page cache and mmap of the new catalog) |

The synthetic FB2 files are small (≈ 2 KB each), so first-call detail/EPUB times for real books (100 KB–5 MB)
are dominated by reading and converting the book; see the fb2conv README for conversion speed.
Web UI check: the built app (`web/dist`, embedded) loaded from the release server in headless Chromium
without JavaScript errors (`web/test-results/screenshots/real-server-*.png`).
