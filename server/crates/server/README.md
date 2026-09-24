# freelib-server

The `freelib-server` binary of the freeLib web edition. It serves the HTTP API
(`docs/web/API.md`), OPDS 1.2, the embedded Svelte app, and runs imports, conversions and
send jobs. It is built on axum 0.8 and tokio. Catalog and `app.db` access go through
`freelib-catalog` (rusqlite, called in `spawn_blocking`). Imports use `freelib-import` and
book conversion uses `freelib-fb2conv`.

## Running

```sh
# development: ./data, ./cache, ./books, ./export are used when /data etc. do not exist
cd web && pnpm build && cd ..                     # optional: the binary embeds web/dist
cd server
FREELIB_BOOKS_DIR=bench-data/files600k \
FREELIB_AUTOIMPORT=bench-data/files600k/lib.inpx \
cargo run --release -p freelib-server             # http://localhost:8080
```

* **Web app.** The web app is embedded from `web/dist` at build time (rust-embed). When `web/dist`
  is missing, an empty folder is embedded instead and `/` answers 404 "web app not built".
  `FREELIB_WEB_DIR=/path/to/dist` serves the app from disk instead, which is useful with
  `pnpm build --watch`. In debug builds rust-embed reads `web/dist` from disk anyway.
* **Front-end development.** For front-end work against the real server, run
  `VITE_API=http://localhost:8080 pnpm dev` in `web/`. The CSRF check accepts the Vite proxy
  because the browser sends `Sec-Fetch-Site: same-origin`.
* **Environment.** The environment variables are listed in `docs/web/ARCHITECTURE.md` under
  "Runtime configuration". Server-specific ones are `FREELIB_BIND`, `FREELIB_TRUST_PROXY` and
  `FREELIB_CALIBRE_TIMEOUT`. `FREELIB_CALIBRE=none` disables Calibre.
* **Open mode.** Without users and without `FREELIB_ADMIN_PASSWORD`, the server runs in open
  mode: nobody logs in and every request acts as the admin (user id 0). Creating the first
  administrator through `POST /users` ends open mode. That first real user takes over the
  shelves, ratings, devices and prefs made in open mode.
* **Admin bootstrap.** With `FREELIB_ADMIN_PASSWORD` set, the user `FREELIB_ADMIN_USER`
  (default `admin`) is created, or its password and admin role are reset, on every start.

Subcommands:

| Command | What |
|---|---|
| `freelib-server` / `freelib-server serve` | run the server |
| `freelib-server healthcheck` | `GET /api/v1/session` on `127.0.0.1:$FREELIB_PORT`; exit 0 on 200, else 1 (Docker `HEALTHCHECK`) |
| `freelib-server migrate-qt <freeLib.sqlite>` | import libraries, shelves (tags) and ratings from the Qt app into `app.db` for the first admin (user 0 in open mode); then re-import each library in the UI |

## Module layout (`src/`)

| Module | Responsibility |
|---|---|
| `main.rs` | CLI, tracing, listener, graceful shutdown (5 s, then exit, because SSE connections never end) |
| `app.rs` | startup: directories, `app.db`, admin bootstrap/open mode, default devices, libraries, `FREELIB_AUTOIMPORT`, cleanup task; router assembly (compression, security headers, trace) |
| `config.rs` | `Config` from the environment; `Config::for_dir` for tests |
| `state.rs` | `AppState`: config, db, per-library `LibRuntime` (catalog handle, import status, cached authors/series JSON and its br/gzip variants), jobs, worker semaphore, caches, login rate limiter, `LibraryDto` |
| `db.rs` | `app.db` access: users, sessions (SHA-256 of the token stored), libraries, devices (+ seeding), shelves, ratings, settings, prefs |
| `auth.rs` | argon2id hashing, cookies, `Auth` / `Admin` extractors (session cache 60 s), credential check with rate limiting |
| `security.rs` | CSRF middleware for `/api` and the security headers |
| `error.rs` | `ApiError` → `{"error", "message"}` |
| `api/*.rs` | `/api/v1` handlers: `session`, `libraries` (+ `/fs`), `browse` (lists, genres, books, search, languages), `books` (detail, cover, file), `shelves` (+ rating), `devices` (+ `/send`, `/fonts`), `jobs` (+ SSE `/events`), `settings` (+ SMTP test, users, prefs) |
| `importer.rs` | import jobs: `freelib_import::import_inpx` in a blocking thread, progress → job/library events, catalog reload and warm-up |
| `jobs.rs` | in-memory job list, cancel flags, produced files, SSE `Event`s with per-user visibility |
| `sender.rs` | `/send` jobs: e-mail (one message per book, `pauseSeconds` between), folder export under `FREELIB_EXPORT_DIR/<target>`, download (single file or zip), `joinSeries` |
| `output.rs` | formats per book, conversion pipeline and cache `cache/out/<lib>/<bookhash>-<profilehash>.<ext>`, download file names |
| `conv.rs` | `Converter` trait: the only place that calls `freelib-fb2conv` |
| `calibre.rs` | `ebook-convert` detection (`--version`) and subprocess with timeout |
| `bookio.rs` | original bytes: seek to `arch_offset` (raw deflate/stored), zip fallback, plain files; rejects `..` in stored paths |
| `preview.rs` | annotation/cover cache (`cache/info`, `cache/covers`), WebP thumbnails (240 px high, libwebp lossy q80) |
| `mail.rs` | SMTP via lettre (rustls; `none` / `starttls` / `tls`) |
| `opds.rs` | OPDS 1.2 feeds, OpenSearch, Basic auth gate, legacy `/opds_<lib>/…` 301 redirects |
| `spa.rs` | embedded or on-disk web app; `/assets/*` immutable, other GETs → `index.html` |
| `util.rs`, `compress.rs` | time, hashes, ETag/304, RFC 5987 `Content-Disposition`, path checks, Accept-Encoding, br/gzip |

## Behaviour where API.md is silent

* **`/fs`.** `path` and the returned `path` / `parent` are relative to `FREELIB_BOOKS_DIR` (`""` is
  the root), as the web client builds them. Symlinks that point outside the books folder are
  hidden. `..` and absolute paths outside the folder get 403.
* **Library paths.** `POST` / `PATCH /libraries` accept `path` and `inpx` either relative to the
  books folder or absolute. They must resolve, after following symlinks, inside
  `FREELIB_BOOKS_DIR`, and they are stored as absolute paths. A bare INPX file name is first
  looked up in the library folder. `FREELIB_AUTOIMPORT` uses the INPX folder as the library
  folder, or a sibling folder with the INPX's name (`lib.inpx` + `lib/`) when one exists.
* **Import mode.** `{mode: "new"}` runs a full rebuild, as described in ARCHITECTURE.md.
* **Caching.**
  * Authors and series lists are cached per catalog version, both as JSON and pre-compressed
    (Brotli quality 5, gzip level 6), with a weak ETag `W/"authors-<lib>-<version>"`.
  * Books, detail, genres and languages responses carry content ETags and answer 304.
  * Covers use their cache file name as the ETag, with `private, max-age=86400`.
  * Downloads carry `private, max-age=3600`.
  * Everything else is compressed by tower-http (br/gzip, level 4). Already-compressed formats
    and images are not compressed again.
* **Formats.**
  * FB2 and EPUB books offer `original, epub, kepub`, plus `azw3, mobi, pdf` when Calibre is
    available. For an EPUB book, `epub` returns the original file.
  * `txt`, `rtf`, `html`, `doc(x)`, `odt`, `mobi`, `azw(3)` and `prc` books can be converted
    with Calibre. Other formats offer only `original`.
  * A format that needs Calibre when Calibre is missing returns 501 `unsupported_format`. An
    unknown format name returns 400.
* **Download names.**
  * The name comes from the device `fileName` template (default `%a - %s %n - %b`) through
    `fb2conv::file_name`. For single files, `/` in the name becomes ` - `; inside zips and
    folder exports, `/` makes sub-folders.
  * KEPUB files are named `*.kepub.epub`.
  * `?device=<id>` takes that device's options, template and default format.
* **Cover.** `size` defaults to `thumb`. `size=full` 404s for books without a cover (including
  books that are not FB2 or EPUB). `size=thumb` never 404s for an existing book: when there is
  no real cover, a generated SVG placeholder (`placeholder.rs`) is returned instead — the same
  look as the SPA's own placeholder (background colour hashed from the title, author line,
  title text) — with `X-Cover: generated` and a public, cacheable `Cache-Control` (it depends
  only on title/author, not on user permissions).
* **Login rate limiting.** After 5 failures from one IP, each further failure doubles the
  lock-out: 1 s, 2 s, … up to 5 min. A locked-out attempt returns 429 with error code
  `rate_limited` and a message that says when to retry. OPDS Basic auth shares the same limiter.
  A successful Basic auth is cached for 10 minutes.
* **CSRF.** These rules apply to state-changing `/api` requests:
  * `Sec-Fetch-Site: cross-site` → 403.
  * An `Origin` that does not match `Host` or `X-Forwarded-Host` → 403. This check is skipped
    when `Sec-Fetch-Site: same-origin` is present.
  * A request body that is not `application/json` → 415.
  * Bodies over 1 MB → 413. `/me/prefs` over 64 KB → 413.
* **Sessions.** Tokens are 32 random bytes. Only their SHA-256 is stored, and the stored hash is
  compared in constant time. Sessions last 30 days. The cookie is
  `HttpOnly; SameSite=Lax; Path=/`, plus `Secure` when `X-Forwarded-Proto: https`. A password
  change logs out all sessions of that user. The last administrator cannot be deleted or
  demoted (409).
* **`newSinceLastVisit`.** A visit starts with the first `GET /session` after 30 minutes of
  inactivity. The count covers books dated after the previous visit.
* **OPDS.**
  * `requireAuth` defaults to `true`, and open mode needs no auth.
  * Authors and series drill down by prefix: letters first, then prefixes of up to 4
    characters while a group has more than 100 entries. The `#` group (non-letters) is paged
    instead.
  * `/opds/:lib/series/<digits>` is a series id.
  * `/opds/:lib/new` lists books from the 30 days before the newest book date in the catalog.
  * Search accepts `q` or the Qt-style `search_string`.
* **Jobs.**
  * Jobs live in memory, so a restart forgets them.
  * Finished jobs and their files are removed after 24 h by a task that runs every 10 minutes.
    That task also removes leftovers in `cache/tmp` and `cache/jobs`.
  * An e-mail device needs `target` (from the device or the request) and configured SMTP,
    otherwise 400.
  * Folder targets are relative sub-folders; `..` gets 400.
* **SSE.**
  * `event: job` goes to the job owner, and import jobs go to all admins.
  * `event: library` goes to everybody, with `newSinceLastVisit` computed for the receiving
    user.
  * A `:ping` comment is sent every 25 s. The stream sends `X-Accel-Buffering: no`.
* **SMTP password.** It is stored in `app.db` in plain text, because it must be usable, and it
  is never returned or logged. `PUT /settings` without `smtp.password` keeps it, and `""` clears
  it.

## Tests

`cargo test -p freelib-server` runs unit tests plus `tests/api.rs`. Each test starts the router over
a temp dir with a generated library (`freelib_import::synth`, real zip archives with FB2 files) and
calls it through `tower::ServiceExt::oneshot`. The tests cover:

* open mode and login mode, roles, rate limiting;
* library create, import, re-import, patch and delete;
* lists (with ETag/304, `?v=` immutable and a Brotli round-trip), books, detail, covers, search,
  languages;
* original, EPUB and KEPUB downloads and the conversion cache;
* ratings and shelves, including per-user isolation;
* devices;
* download, zip, joined-series and folder-export jobs, and SSE delivery;
* `/fs` traversal and symlink rejection, CSRF, body limits;
* OPDS feeds, with well-formedness and OPDS rel/type checks, and the legacy redirects;
* Calibre formats, using a fake `ebook-convert` shell script;
* SMTP test and Send to Kindle, against an in-process fake SMTP server;
* SPA serving.
