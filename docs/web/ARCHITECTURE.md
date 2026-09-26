# freeLib Web — architecture

The web edition of freeLib: a Rust server with a Svelte single-page app, shipped
as a Docker image. The Qt desktop app in `freeLib/` stays as is; the web
edition lives in `server/`, `web/`, `docker/`.

```
Browser (Svelte 5 SPA) ──HTTP/JSON + SSE──▶ freelib-server (Rust, axum)
                                             ├─ app.db             users, sessions, libraries, devices, shelves, ratings, settings, API tokens, history
                                             ├─ ratings.db         Open Library rating cache (safe to delete)
                                             ├─ rating worker      Open Library lookups, ≥ 1 s apart, priority queue
                                             ├─ secret.key         key of the encrypted secrets in app.db (unless FREELIB_SECRET_KEY[_FILE])
                                             ├─ /mcp               MCP server (rmcp, streamable HTTP, API tokens or OAuth access tokens)
                                             ├─ /oauth/*           OAuth 2.1 authorization server for MCP clients (+ /.well-known metadata)
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
| `FREELIB_CONTACT_EMAIL` | unset | Contact address in the User-Agent of Open Library requests (`freeLib/<version> (+https://github.com/Kwull/freeLib; <email>)`), as Open Library asks of API users |
| `FREELIB_OPENLIBRARY_URL` | `https://openlibrary.org` | Base URL of Open Library (tests point it at a local fake) |
| `FREELIB_MCP_RATE` | `120` | MCP requests per API token (or authorized app) and minute |
| `FREELIB_SECRET_KEY` | unset | Key of the secrets stored in `app.db`: 32 bytes as 64 hex digits or base64 |
| `FREELIB_SECRET_KEY_FILE` | unset | File with the key (Docker secret); without both, `<data dir>/secret.key` is generated (0600) |
| `FREELIB_SECRET_KEY_OLD` | unset | Previous key during a key change; stored secrets are re-encrypted at start |
| `FREELIB_OAUTH_CLIENT_HOSTS` | `claude.ai, claude.com` | Hosts whose `https://` redirect URIs and Client ID Metadata Documents OAuth clients may use (`*.x` sub-domains, `*` any public host); loopback redirects are always allowed |
| `RUST_LOG` | `info` | Logging |

Admin settings (Settings → Server, stored in `app.db` `setting`): `externalRatings.enabled` (default true — when
false nothing is sent to Open Library) and `mcp.enabled` (default true — when false `/mcp` answers 403).

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
  arch_csize INTEGER, arch_method INTEGER,
  work_id INTEGER NOT NULL DEFAULT 0  -- id of the first book of the same work (editions, see "Search and editions")
);
CREATE TABLE book_author (book_id INTEGER NOT NULL, author_id INTEGER NOT NULL,
  pos INTEGER NOT NULL,        -- author order in the INPX record (0 = first author)
  PRIMARY KEY (author_id, book_id)) WITHOUT ROWID;
CREATE TABLE book_genre  (book_id INTEGER NOT NULL, genre_id INTEGER NOT NULL, PRIMARY KEY (genre_id, book_id)) WITHOUT ROWID;
CREATE TABLE genre_count (genre_id INTEGER PRIMARY KEY, count INTEGER NOT NULL);  -- live books; groups: distinct books in the group
CREATE TABLE lang_count (lang TEXT PRIMARY KEY, count INTEGER NOT NULL) WITHOUT ROWID;  -- live books per language
CREATE TABLE letter_index (kind TEXT NOT NULL, letter TEXT NOT NULL, count INTEGER NOT NULL, first_pos INTEGER NOT NULL, PRIMARY KEY (kind, letter)) WITHOUT ROWID;
-- FTS rows hold normalize()d text; rowid = book / author / series id. `stems` = Snowball stems of the title, author and
-- series words that differ from the word; `latin` = their Latin keys (transliteration, variants folded) that differ.
CREATE VIRTUAL TABLE book_fts USING fts5(title, authors, series, keywords, stems, latin, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2', prefix='2 3');
CREATE VIRTUAL TABLE author_fts USING fts5(name, stems, latin, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE series_fts USING fts5(name, stems, latin, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2');
CREATE TABLE vocab (word TEXT PRIMARY KEY, freq INTEGER NOT NULL) WITHOUT ROWID;  -- title/author/series words (≥ 3 chars) → live books
-- indexes (created after bulk load)
CREATE INDEX author_sort ON author(sort_key);
CREATE INDEX series_sort ON series(sort_key);
CREATE INDEX book_series ON book(series_id, serno);
CREATE INDEX book_first_author ON book(first_author_id);
CREATE INDEX book_date ON book(date);
CREATE INDEX book_lang ON book(lang);
CREATE INDEX book_ba_rev ON book_author(book_id, pos);
CREATE INDEX book_bg_rev ON book_genre(book_id);
CREATE INDEX book_work ON book(work_id);
```

Catalog schema version 3 (stems, Latin keys, `vocab`, `work_id`); older catalogs are rebuilt at start.

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

## Search and editions

`freelib_catalog::text` (stemming, Latin keys, edit distance, work keys), `search.rs`, `vocab.rs`, `works.rs`, `home.rs`.

* **Word forms.** Every title/author/series word is stemmed at import (Snowball Russian for Cyrillic, English
  (Porter2) for Latin, a small suffix stripper for words with `і ї є ґ` — Snowball has no Ukrainian); stems that differ
  from the word go to the `stems` column. A query word of ≥ 3 letters also matches its stem exactly, so `книгу`,
  `книгой` find `Книга`, `Книги`, and `стругацкие` finds `Стругацкий`.
* **Transliteration.** `latin` holds each word's Latin key — the Open Library matcher's `word_key`, moved into the
  catalog: Russian/Ukrainian → Latin, accents folded, `iy/ii/yi → y`, `ts → c`, `kh → h`, `ks → x`, … — when it differs
  from the word. A query word of ≥ 3 letters also matches its own key (a prefix from 4 letters, exact for 3), in all
  columns, so `strugatsky`, `Strugatskii`, `strugackie` find `Стругацкий`, `лем` finds `Lem`, and a Latin-script record
  written `Strugatsky` is found by `Strugatskii` too. `ё = е` comes from `normalize`.
* **One FTS query**: every word becomes `("word"* OR "stem" OR "key"*)`, AND-ed; a second query with plain prefixes and a
  third with the phrase on the title column (`title : "война и мир"*`, one word: `title : ^ "word"*`) assign tiers:
  phrase 3 > all prefixes 2 > word forms / transliterations 1; the score is `bm25 − 1000 × tier`, so the existing
  relevance/rating ordering code is unchanged. Authors/series: name starts with the query, then all prefixes, then
  the rest, each by book count.
* **Typos.** When a search finds fewer than 3 matches, each word of ≥ 4 letters that is neither the prefix of a
  vocabulary word nor (by its key) of a word's key is replaced by the closest vocabulary word (optimal string alignment,
  ≤ 1 edit up to 7 letters, ≤ 2 from 8, in the word's own script and in Latin-key space, then by frequency). A 64-bit
  character-set signature and the length pre-filter candidates. Nothing found → the corrected query is searched
  (`corrected`); something found → offered (`didYouMean`) when it finds more. The vocabulary is held in memory per
  catalog (loaded in the warm-up: arenas + 40 bytes/word).
* **Highlighting**: the server returns the normalized words of the shown names that matched (prefix, stem or key);
  the SPA marks whole words whose normalized form is in that set (`web/src/lib/utils/highlight.ts`).
* **Editions** (`works.rs`): `work_id` = the first book with the same language, `work_title_key(title)` (normalized,
  trailing edition notes such as `(другой перевод)`, `[иллюстрации]`, `(пер. …)`, `(СИ)` dropped; other brackets kept)
  and author-id set; unknown authors and generic titles ("Избранное", "Рассказы", …) keep their own id. Lists group
  in memory (`BookAttrs` now also holds size and work id, +8 bytes/book), each work at the position of its first edition;
  the best copy: not deleted > known cover > FB2 > EPUB > other > larger (20 % buckets, ≤ 30 MB) > newer > library
  rating > lower id. Covers are known per library and `book_key` from preview extraction since the server started
  (`find::CoverHints`, not persisted) and passed as `RatingSource::has_cover`. Grouped pages are cut by offset from
  the grouped selection. Search groups after ranking; MCP `search_books` always groups.

### Start page

`GET …/home` (`server/src/find.rs`, catalog `home.rs`): the user's history (`book_history`: latest time per book),
ratings, shelves, follows and dismissed series are resolved to ids once. **Continue series**: for each series of a
book done (history or rated), the series' works in series order (editions grouped; in a publisher series with more
than 3 first authors only the books sharing an author with the user's books there), the next ≤ 2 works after the last
one done that are not done; finished and dismissed series are dropped; ordered by latest activity. **New from authors**:
live books dated ≥ since (previous visit, or the chosen window) by followed authors, in followed series, or by the
authors of books done, rated ≥ 4 or shelved (not anthologies, not "Автор неизвестен"), minus works the user has,
grouped, newest first. **Empty state**: the best library-rated works of the 30 days before the newest book.

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

Migration v4: `api_token(id, user_id, name, token_hash UNIQUE, prefix, scopes, created_at, last_used_at, expires_at)`
(SHA-256 hex of the secret; the secret `fl_` + base64url of 32 random bytes is shown once), `api_audit(id, user_id,
token_id, tool, ok, detail, at)` (≤ 500 per user kept, the UI shows 50), and `book_history(id, user_id, library_id,
book_key, action ∈ send|download|read, device, at)` (≤ 5000 per user; the same book and action within an hour is
recorded once). History is written by the send/export/download jobs when they succeed (`send` for e-mail and folder
devices, `download` for download devices) and by `GET …/file` (`read` for the web reader's `inline=1`, else `download`).
Migration v5: `device_order(user_id, device_id, pos)`: each user's order of the devices they see (own + shared).
`GET /devices` sorts by it (devices never ordered follow by id); **the first device is the user's default** everywhere
(quick send in the details pane and the selection bar, the Send dialog's preselection, MCP `device: "default"`). A
first-in-order rule was chosen over "last used": it is explicit and stable — a one-off download no longer changes
where the next "Send" goes. The old `lastDevice` UI pref is no longer read.
Migration v6 (OAuth): `oauth_client(client_id, name, redirect_uris JSON, client_uri, created_at, last_used_at)`
(dynamically registered clients), `oauth_grant(id, user_id, client_id, client_name, client_kind ∈ cimd|dcr,
redirect_uri, scopes, resource, created_at, last_used_at)` (one per authorization = one "authorized app"),
`oauth_token(token_hash, grant_id → oauth_grant ON DELETE CASCADE, kind ∈ access|refresh, scopes, expires_at,
rotated_at)` (SHA-256 of the secret only; unix times), and `api_audit.grant_id`.
Tokens, audit rows, history, device order and OAuth grants are removed with their user; open-mode data (user 0) is
adopted by the first account, except OAuth grants made in open mode, which are deleted.

### Secrets at rest

`server/src/secrets.rs`. Values the server must read back (today `setting.smtp.password`; every such field is listed
in `secrets::SECRET_FIELDS`) are stored as `enc:v1:<key id>:<base64url(nonce ‖ ciphertext ‖ tag)>`:
XChaCha20-Poly1305, a random 192-bit nonce per value, associated data `freelib/v1/<field name>` (a value moved to
another field does not decrypt), key id = first 4 bytes of SHA-256 over the key (says which key encrypted a value
without revealing it). The key: `FREELIB_SECRET_KEY`, else `FREELIB_SECRET_KEY_FILE`, else `<data dir>/secret.key`
(created with `O_EXCL` and mode 0600 on the first start, with a log line asking to back it up with `app.db`; looser
permissions are tightened). At start `secrets::migrate` runs in one transaction: plain-text values are encrypted,
values of `FREELIB_SECRET_KEY_OLD` are re-encrypted with the current key, and a value encrypted with an unknown key
(or failing authentication) stops the start with an explanation; `freelib-server forget-secrets` removes the stored
secrets when the key is lost. The settings API encrypts the SMTP password before it reaches the database;
`SmtpConfig::revealed` decrypts it just before sending. Keys and secrets never appear in logs or `Debug` output
(`secrets::Redacted` wraps `FREELIB_ADMIN_PASSWORD`, `FREELIB_OIDC_CLIENT_SECRET` and the keys in `Config`).

Why not encrypt everything: login passwords are Argon2id hashes and session / API / OAuth tokens SHA-256 hashes of
256-bit random values. The server only compares them, so a one-way hash is strictly better than reversible
encryption: nothing — not even the key — turns the database back into a working credential. OIDC pending sign-ins,
OAuth authorization codes and consent requests live only in memory. The key file next to `app.db` protects leaked
database copies and backups, not a full compromise of the data volume; the environment / Docker secret options
keep the key out of that volume.

Migration v6: `follow(user_id, library_id, kind ∈ author|series, key, name, created_at)` and
`series_dismiss(user_id, library_id, key, name, at)`, keyed by the author's / series' normalized name (`sort_key`,
the importer's dedup key) because ids are not stable across imports.
Tokens, audit rows, history, device order, follows and dismissed series are removed with their user; open-mode data (user 0) is adopted by the first account.

User data is keyed by `(library_id, book_key)`, so it survives re-imports.

## Ratings

Three sources per book, all exposed on `Book`:

* **my rating** — `rating` in `app.db` (0–5);
* **library rating** — the INPX `LIBRATE`/`STARS` field, clamped 0–5 at import (Flibusta's own ratings are 0–5), stored
  in `book.stars`, API `libRating` (0 = none);
* **external rating** — Open Library (`server/src/extrating/`).

### Open Library enrichment

* **Lookup** (`extrating/openlibrary.rs`): `GET /search.json?title=…&author=<surname>&fields=key,title,author_name,ratings_count,edition_count`,
  then `GET <work>/ratings.json` (`summary.average`, `summary.count`). Matching is conservative — no match beats a
  wrong one: the book title (bracketed notes like `(сборник)` dropped) must equal the result title, or one of them its
  main title before a `:`, ` - ` or `. ` (≥ 4 letters), compared as a Latin key (Cyrillic transliterated, accents folded,
  `iy/ii/yi → y`, `kh → h`, `ts → c`, `ks → x`, … so `Strugatsky = Strugatskii = Стругацкий`); **and** one of the
  book's author surnames (INPX last names; "Автор неизвестен" is never used) must be a word of one of the result's
  author names. Among several matches the one with most ratings wins. Cyrillic books are tried as is, then with a
  transliterated surname, then with transliterated title and surname (≤ 3 searches, stopping at the first match).
  Titles with fewer than 2 key characters or books without an author are not looked up.
* **Cache**: `ratings.db` (WAL) in the data directory, separate from `app.db` because it is written about once a second
  and can be deleted at any time: `ext_rating(library_id, book_key, source, status found|not_found|error, average,
  count, work_key, fetched_at, attempts, message)` keyed by (library, `book_key`, source) — `book_key` alone is only
  unique per library (`lib:<LIBID>` of two different collections would collide) — and `sweep(library_id, next_id,
  done_at)`. Found ratings are refreshed after 90 days, `not_found` after 180 days, errors retried after a day; a
  failed refresh keeps the previous answer.
* **Memory**: at start the found rows are loaded into `book_key → (average×100, votes)` per library (250k cached rows:
  the server is listening 0.2 s after start). Book lists read it per row. For rating sorts and filters a dense array
  indexed by book id (6 bytes/book, 3.3 MB for 551k books) is built per catalog version during the catalog warm-up
  (≈ 0.5 s for 150k found ratings) and updated in place as lookups arrive.
* **Worker** (`ExtRatings::run`, one task): one request at a time, at least 1 s apart (`Limiter`); HTTP 429/5xx or
  network errors pause all requests 30 s, doubling up to 1 h, reset by a success. Priorities: (1) books the user
  opened (detail), shelved, rated or sent — `Priority::User`; (2) books of the author/series pages the user browses
  (first 300 of a page) — `Priority::Browse`; (3) a sweep over every library by book id, skipping fresh rows; after a
  full pass it restarts a day later (which picks up refreshes). Queues are bounded (2 000 / 10 000, oldest dropped),
  enqueuing never blocks a request. With `externalRatings.enabled` off nothing is queued or sent; MCP
  `get_external_rating` looks one book up immediately (same limiter) or returns the cached answer.
* **Privacy**: requests carry book titles and author surnames only, never anything about users, with the User-Agent
  above. Progress (looked up / total, found, with ratings, queue, pause, last error) is shown in Settings → Server and
  per library on the Libraries page.

### Rating filters and sorts

`freelib_catalog::rank`: `RatingQuery {sort: None|My|Lib|Ext, min_my, min_lib, min_ext, min_ext_votes, unrated_by_me,
kids_max_age}` and a `RatingSource` trait (my rating and external rating by book id) that the server implements per
request (the user's ratings of the library resolved to ids; the dense external array). `Catalog::books_rated` = `books`
without a rating query; with one, the selection's ids come from `BookAttrs` (genre, since — now also holding
`stars` and the age estimate, +2 bytes/book) or one SQL query (author, series, shelf), are filtered, stably sorted
(best first, unrated last) and paged by offset. `search_rated` applies the filters before facets and sorts by the
rating instead of bm25 (relevance breaks ties). The web app sorts and filters author/series lists (loaded in full) in
the browser with the same rules, and sends the parameters for genres, new arrivals, shelves and search.

### Kids' age estimate (heuristic)

`freelib_catalog::kids::age_for(genre ids, keywords)` → 0, 6, 12, 16, 18 or unknown. It is **a heuristic, not an age
rating**: nobody reviewed the books, and a children's genre on an adult book (or no genre) gives a wrong or missing
estimate. Rules, strongest first: adult genres (`love_erotica`, `love_hard`, `home_sex`) or adult words in the INPX
keywords (`18+`, `эротика`, `erotica`, `порно`) → 18+; an explicit `0+`/`6+`/`12+`/`16+` keyword → that; mature
genres (horror, thrillers, serial killers, romance, counterculture, sex psychology, hard-boiled) → 16+; children's
genres → the highest of 0+ (tales, nursery verse, children's folklore), 6+ (children's prose/classics/education, folk
tales, fairy fantasy), 12+ (children's adventure/mystery/SF, young adult, gamebooks); children's / teen words in the
keywords → 6+ / 12+; else unknown. The filter "suitable for age ≤ N" excludes unknown books.

## MCP server

`server/src/mcp/` — the official Rust SDK `rmcp` (3.4) `StreamableHttpService` mounted at `/mcp` (outside `/api/v1`,
no cookies), in stateless mode with JSON responses (`legacy_session_mode = false`, `NeverSessionManager`); protocol
versions up to 2026-07-28. Its own Host/Origin checks are off because `host_guard` (`FREELIB_ALLOWED_HOSTS`) already
runs and the endpoint needs a bearer token. A middleware (`mcp::gate`) checks `mcp.enabled`, the token
(`tokens::authenticate`: SHA-256 lookup, constant-time compare, expiry, 60 s cache cleared on revocation or user
changes), the per-token rate limit (fixed one-minute window), and passes the token to the handler in the request
extensions. The handler implements `list_tools` / `call_tool` / `list_prompts` / `get_prompt` itself: the tool table
(`mcp/tools.rs`) holds each tool's scope, description and argument JSON schema (`schemars`); `tools/list` shows the
tools the token may use, every call is checked again and written to `api_audit`. Tools answer with
`structuredContent` (and the same JSON as text). Server instructions describe the library, the ids, the three
ratings, the age heuristic and the suggested workflow. Scopes: `read` (catalog, own shelves/ratings/history/devices,
suggestions, Open Library lookups), `write` (shelves, ratings), `send` (`send_books`, `get_job`).

`suggest_candidates` (`mcp/suggest.rs`) scores server-side: seeds are the given books and/or the profile (rated 5 → 3,
4 → 2, 3 → 0.5, ≤ 2 → −2; on a shelf +1; sent/downloaded/read +1; anthologies do not seed authors). Candidates: the
books of the 15 best-weighted authors, of the seed series, and up to 1 500 well-rated books (library ≥ 4 or Open
Library ≥ 4.0 with ≥ 5 votes) of the 5 top genres; read/rated/shelved books are excluded by default. Score:
same author `3 × w/max`, next unread number of a series `+5` (later numbers `+2`), shared genres up to `+2`, library
rating `(stars − 3) × 0.5`, Open Library `(avg − 3.5)`, disliked author down to `−3` — each part explained in `reasons`.

### OAuth for MCP clients

`server/src/oauth/` (flows, `cimd.rs`, `store.rs`) and `api/oauth.rs` (consent page API, authorized apps). freeLib is
its own authorization server following the MCP authorization spec (2025-11-25 / 2026-07-28) and Claude's connector
requirements, so claude.ai, Claude Desktop/mobile and Claude Code connect by signing in. Enabled when
`FREELIB_PUBLIC_URL` is an `https://` origin (or `http://localhost`); the issuer is that URL and the only resource is
`<issuer>/mcp`.

* **Discovery**: `/mcp` without a valid token → `401` + `WWW-Authenticate: Bearer realm="freeLib",
  resource_metadata="<issuer>/.well-known/oauth-protected-resource/mcp", scope="read write send"` (+ `error=
  "invalid_token"` when a token was sent). `/.well-known/oauth-protected-resource[/mcp]` (RFC 9728: `resource`,
  `authorization_servers`, `scopes_supported`), `/.well-known/oauth-authorization-server` (RFC 8414: endpoints,
  `code_challenge_methods_supported: ["S256"]`, `token_endpoint_auth_methods_supported: ["none"]`,
  `client_id_metadata_document_supported`, `authorization_response_iss_parameter_supported`). No
  `openid-configuration`: freeLib issues no ID tokens, and MCP clients try RFC 8414 first.
* **Clients** are public (PKCE, no secrets). *Client ID Metadata Documents*: a `client_id` that is an `https://` URL
  with a path on a trusted host (`FREELIB_OAUTH_CLIENT_HOSTS`) is fetched (public addresses only, connection pinned
  to the checked address, no redirects, 5 s, 64 KiB, cached per `max-age` 1 min … 1 day); its `client_id` must equal
  the URL. Claude's two documents are built in as a fallback. *Dynamic Client Registration* (`POST /oauth/register`,
  RFC 7591, deprecated by MCP but still used by many clients): 20 per address and hour, at most 500 clients; clients
  never used are removed after a day, unused ones without grants after 90 days. Redirect URIs: `https://` on a trusted
  host or `http://127.0.0.1|[::1]|localhost` (RFC 8252); matching is exact, except that the port of a loopback URI
  is ignored.
* **Authorization** (`GET /oauth/authorize`): an unknown client or unregistered redirect URI is never redirected to
  (the browser goes to `/oauth/consent?error=…`); then `response_type=code`, PKCE `S256`, known scopes
  (`offline_access` ignored, none = all) and `resource` (RFC 8707, `<issuer>/mcp` or `<issuer>`) are checked, errors
  redirected with `state` and `iss`. A pending request (15 min, ≤ 30 per address) is created and the browser goes to
  the SPA's consent page `/oauth/consent?request=<id>`. The page (after the normal sign-in, password or SSO) reads
  `GET /api/v1/oauth/requests/{id}` (app name, the verified host of a CIMD client id, redirect host, loopback warning,
  scopes, a per-request CSRF token) and posts the decision with the chosen subset of scopes; the API's CSRF layer
  applies (JSON only, `Origin`, `Sec-Fetch-Site`), and `frame-ancestors 'self'` prevents clickjacking. The answer is
  the redirect URL with `code` (32 random bytes, 5 min, single use, in memory) or `error=access_denied`, `state` and
  `iss`; the decision is written to the audit log.
* **Tokens** (`POST /oauth/token`, form-encoded, `Cache-Control: no-store`): `authorization_code` needs the same
  client, `redirect_uri`, the PKCE verifier and (if sent) the same resource; a second use of a code revokes the grant
  made from it. It creates a grant with an access token `flo_…` (1 h) and a refresh token `flr_…` (90 days), both
  stored as SHA-256. `refresh_token` rotates (the old one is marked `rotated_at` and kept until it expires); a rotated
  refresh token presented again revokes the whole grant (reuse detection) and is audited; an optional narrower
  `scope` applies to the new access token. Errors are RFC 6749 codes (`invalid_grant`, `invalid_client`, …).
  `POST /oauth/revoke` (RFC 7009): a refresh token revokes its grant, an access token only itself; always 200.
* **Resource server**: `tokens::authenticate` accepts `fl_` API tokens and `flo_` access tokens (unexpired, grant and
  user exist, grant resource = the current `<issuer>/mcp` — audience binding). The 60 s cache never outlives the access
  token and is cleared on every revocation. For OAuth tokens the gate reads the JSON-RPC body: a `tools/call` of a tool
  outside the granted scopes gets `403` + `WWW-Authenticate: Bearer error="insufficient_scope", scope="<granted +
  needed>"` (step-up); `tools/list` shows only the granted tools. Tool calls are audited with the grant; the per-token
  rate limit applies per grant.
* **Settings → Account → API tokens & MCP** lists the authorized apps (`GET /me/oauth/apps`: name, verified host,
  redirect host, scopes, created, last used) with revoke (`DELETE /me/oauth/apps/{id}`), and shows the connector
  steps. Cleanup (every 10 min): expired tokens, grants without tokens, stale clients, expired in-memory state.

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

### Measured: search, editions, start page (catalog layer, 600k synthetic library)

`bench` on a freshly generated `gen-inpx --books 600000` (600k records, 552,088 live, 179,592 authors, 61,696
series; the generator now also marks some famous-title editions `(другой перевод)` / `[иллюстрации]`), same 4-core VM,
warm cache, milliseconds:

| Operation | p50 | p95 | max |
|---|---|---|---|
| Import (schema 3: stems, Latin keys, vocabulary, work ids) | | | 31.4 s (was 16.7 s); catalog 427 MB |
| Search, 25 queries × 3 (kind=all, facets) | 29.6 | 86.3 | 96.7 |
| Search, same + 16 word-form / transliteration / typo queries, grouped by work (123 runs) | 22.3 | **97.3** | 126.9 |
| Word forms / transliteration only (`книгу`, `мирами`, `strugatsky`, `azimov`, `tolstoi`, `dark towers`…) | 11.6 | 67.4 | 82.5 |
| Typos (`азимв`, `sheckley robrt`, … incl. the correction and the second search) | 8.9 | 13.7 | 13.7 |
| Vocabulary load (once per catalog, in the warm-up) | | | < 1 (21,875 words, 1 MB; the synthetic vocabulary is small — a real Flibusta catalog has a few 100k words, est. ≈ 20 MB, ≈ 0.3 s) |
| Grouped books of an author (400 authors, top 5,335 books) | 0.11 | 12.8 | 51.7 |
| Grouped new arrivals, 30 days / all 552k books | 11.1 / 153.7 | 12.6 / 156.3 | |
| Grouped biggest top-level genre | 47.6 | 49.3 | |
| Start page: continue series (300 books read) / new from authors (30 days) | 22.3 / 20.9 | 34.4 / 28.5 | |

Search p95 stays well under the 300 ms target. Reproduce: `gen-inpx --books 600000 --out bench-data/synthetic-600k.inpx`,
then `bench --inpx bench-data/synthetic-600k.inpx`.

### Measured: ratings (release server, 600k synthetic library)

`bench-data/synthetic-600k.inpx` (551,652 live books), 5 000 own ratings, 250 000 cached Open Library rows (150 000
found with votes); `curl`-style timings over HTTP on localhost, 7 runs, p50 / max:

| Request | p50 | max |
|---|---|---|
| all 551k books (`since=1900-01-01`), default order, first page | 206 ms | 226 ms |
| all 551k, `sort=ext` (index built at warm-up; 528 ms if not) | 46 ms | 47 ms |
| all 551k, `sort=lib` / `sort=my` | 43 / 60 ms | 101 / 106 ms |
| all 551k, `minExt=4&minExtVotes=100` / `unratedByMe=1` / `kidsMaxAge=12` | 33 / 47 / 28 ms | 34 / 48 / 29 ms |
| top-level genre (103,025 books) default / `sort=ext` | 14 / 20 ms | – / 22 ms |
| same genre `sort=lib&minLib=3`, page at offset 5 000 | 17 ms | 18 ms |
| leaf genre (8 351) `sort=my` | 18 ms | 19 ms |
| biggest author (1 621 books, all) `sort=ext` | 24 ms | 36 ms |
| search `сер` (53,805 hits) default / `sort=ext&minLib=2` | 54 / 56 ms | 105 / 57 ms |

RSS after these: ≈ 325 MB (≈ 285 MB right after start with the cache loaded).

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
