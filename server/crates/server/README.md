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
  "Runtime configuration". Server-specific ones are `FREELIB_BIND`, `FREELIB_TRUST_PROXY`,
  `FREELIB_ALLOWED_HOSTS`, `FREELIB_CACHE_MAX_MB` and `FREELIB_CALIBRE_TIMEOUT`.
  `FREELIB_CALIBRE=none` disables Calibre.
* **Open mode.** Without users and without `FREELIB_ADMIN_PASSWORD`, the server runs in open
  mode: nobody logs in and every request acts as the admin (user id 0). Creating the first
  administrator through `POST /users` ends open mode. That first real user takes over the
  shelves, ratings, devices and prefs made in open mode. Open mode answers only requests for
  `localhost`, IP literals and `FREELIB_ALLOWED_HOSTS` (421 otherwise): a web page cannot use DNS
  rebinding to reach it as "admin".
* **Admin bootstrap.** With `FREELIB_ADMIN_PASSWORD` set, the user `FREELIB_ADMIN_USER`
  (default `admin`) is created, or its password and admin role are reset, on every start.

Subcommands:

| Command | What |
|---|---|
| `freelib-server` / `freelib-server serve` | run the server |
| `freelib-server healthcheck` | `GET /api/v1/session` on `127.0.0.1:$FREELIB_PORT`; exit 0 on 200, else 1 (Docker `HEALTHCHECK`) |
| `freelib-server forget-secrets` | remove the encrypted secrets (SMTP password) from `app.db`: the way out when the secret key is lost |
| `freelib-server migrate-qt <freeLib.sqlite>` | import libraries, shelves (tags) and ratings from the Qt app into `app.db` for the first admin (user 0 in open mode); then re-import each library in the UI |

## Module layout (`src/`)

| Module | Responsibility |
|---|---|
| `main.rs` | CLI, tracing, listener, graceful shutdown (5 s, then exit, because SSE connections never end) |
| `app.rs` | startup: directories, `app.db`, admin bootstrap/open mode, default devices, libraries, `FREELIB_AUTOIMPORT`, cleanup task; router assembly (compression, security headers, trace) |
| `config.rs` | `Config` from the environment; `Config::for_dir` for tests |
| `state.rs` | `AppState`: config, db, per-library `LibRuntime` (catalog handle, import status, `deleted` flag, cached authors/series JSON and its br/gzip variants, `newSinceLastVisit` counts), jobs, worker / preview / password-verification semaphores, caches, login rate limiter (per address and per user name), `LibraryDto`, `catalog_call` (retries once on a catalog replaced mid-request) |
| `db.rs` | `app.db` access: users, sessions (SHA-256 of the token stored), libraries, devices (+ seeding, per-user order), shelves, ratings, settings, prefs, API tokens + audit, book history |
| `auth.rs` | argon2id hashing, cookies, `Auth` / `Admin` extractors (session cache 60 s), credential check with rate limiting |
| `oidc.rs` | OpenID Connect sign-in: discovery/JWKS cache, authorization code + PKCE + state + nonce, ID token validation, pending sign-ins, account lookup/creation/linking by (issuer, subject), admin group (see docs/web/ARCHITECTURE.md "Single sign-on") |
| `security.rs` | CSRF middleware for `/api`, the `Host` check (DNS rebinding), security headers and the Content-Security-Policy |
| `cache.rs` | LRU eviction of `cache/{out,covers,info}` (`FREELIB_CACHE_MAX_MB`), temp-file cleanup at startup |
| `error.rs` | `ApiError` → `{"error", "message"}` |
| `api/*.rs` | `/api/v1` handlers: `session`, `libraries` (+ `/fs`), `browse` (lists, genres, books, search, languages), `books` (detail, cover, file), `shelves` (+ rating), `devices` (+ `/send`, `/fonts`), `jobs` (+ SSE `/events`), `settings` (+ SMTP test, users, prefs), `oidc` (`/auth/oidc/*`, `/me/account`, `/me/password`, `/me/oidc`) |
| `extrating/` | Open Library ratings: `openlibrary.rs` (HTTP trait, rate limiter with backoff, search + conservative title/surname matcher with transliteration, `ratings.json`), `mod.rs` (`ratings.db` cache, in-memory index + dense per-catalog arrays, priority queue, background worker, on-demand lookup) |
| `tokens.rs` | personal API tokens: `fl_` secrets, SHA-256 storage, scopes, bearer authentication (60 s cache), per-token rate limit; `api/tokens.rs` = `/me/tokens` |
| `secrets.rs` | encryption at rest of reversible secrets (XChaCha20-Poly1305, `enc:v1:<key id>:…`), key loading (`FREELIB_SECRET_KEY[_FILE]`, generated `secret.key`), start-up migration / rotation / wrong-key refusal |
| `oauth/` | OAuth 2.1 authorization server for MCP clients: `mod.rs` (metadata documents, `/oauth/authorize`, `/token`, `/register`, `/revoke`, redirect URI policy, PKCE, resource binding, in-memory consents and codes, rate limits, cleanup), `cimd.rs` (Client ID Metadata Documents with SSRF guards), `store.rs` (`oauth_client` / `oauth_grant` / `oauth_token`); `api/oauth.rs` = consent page API and `/me/oauth/apps` |
| `mcp/` | MCP server at `/mcp` (rmcp streamable HTTP, stateless): `mod.rs` (gate middleware, `ServerHandler`, prompts, instructions), `tools.rs` (17 tools with scopes and schemas), `suggest.rs` (candidate scoring) |
| `find.rs`, `api/find.rs` | start page (`/home`: continue series, new from authors, picks), follows and dismissed series (`app.db` v6), editions endpoint, known covers (`CoverHints`) for the best copy |
| `importer.rs` | import jobs: `freelib_import::import_inpx` in a blocking thread, progress → job/library events, catalog reload and warm-up |
| `jobs.rs` | in-memory job list, cancel flags, produced files, SSE `Event`s with per-user visibility |
| `sender.rs` | `/send` jobs: e-mail (one message per book, `pauseSeconds` between), folder export under `FREELIB_EXPORT_DIR/<target>`, download (single file or zip), `joinSeries` |
| `output.rs` | formats per book, conversion pipeline and cache `cache/out/<lib>/<bookhash>-<profilehash>.<ext>`, download file names |
| `conv.rs` | `Converter` trait: the only place that calls `freelib-fb2conv` |
| `calibre.rs` | `ebook-convert` detection (`--version`, ≥ 6.19 required) and subprocess in a private temp dir with timeout and cancellation |
| `bookio.rs` | original books as bounded streams: seek to `arch_offset` (validated against the file; raw deflate/stored), zip fallback with the importer's name rules, plain files; 256 MiB limit; rejects `..` in stored paths |
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
  * Other formats offer only `original`: Calibre only ever gets EPUB input (its HTML/TXT/DOCX
    input plugins are a large attack surface, see CVE-2023-46303).
  * `original` (and `epub` of an EPUB) are streamed from the archive, never loaded whole.
  * `inline=1` only applies to EPUB; HTML/XHTML/XML/FB2/SVG are sent as
    `application/octet-stream`; files and covers carry `Content-Security-Policy: sandbox`.
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
* **Login rate limiting.** Two independent keys: the client address (IPv6 per /64; behind
  `FREELIB_TRUST_PROXY` `X-Real-IP` or the rightmost `X-Forwarded-For` entry) and the lower-cased
  user name. After 5 failures of a key, each further failure doubles its lock-out: 1 s, 2 s, … up
  to 5 min. Attempts in flight count as failures until they finish, and at most two per key run
  at once. A locked-out attempt returns 429 with error code `rate_limited` and a message that says
  when to retry. At most two Argon2 verifications run at a time server-wide. OPDS Basic auth
  shares all of this. A successful Basic auth is cached for 10 minutes.
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
  inactivity; the previous visit is stored in `user_state.prev_visit`. INPX dates have no time,
  so the count covers books dated on or after the server-local (`TZ`) day of the previous visit.
* **Users.** Names are unique case-insensitively (409). Ids are never reused (high-water mark in
  `setting.last_user_id`); deleting a user cancels and removes its jobs and files and closes its
  event streams.
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
  * At most 5 queued + running send/export/download jobs per user (429). Only finished jobs are
    trimmed when more than 1000 are kept. Cancelling kills a running Calibre process.
  * An e-mail device needs `target` (from the device or the request) and configured SMTP,
    otherwise 400. The target must match `smtp.allowedRecipients` (403) and the user's mails
    of the day may not exceed `smtp.dailyLimitPerUser` (429); both apply to admins too.
  * Folder targets are relative sub-folders; `..` gets 400. Only admins create or change folder
    devices; readers use shared ones as configured. Exports never overwrite (` (2)` suffix).
* **SSE.**
  * `event: job` goes to the job owner, and import jobs go to all admins.
  * `event: library` goes to everybody: the DTO is built once per event, `newSinceLastVisit`
    is filled in per receiving user (cached per catalog version and day).
  * Before each event the user is re-checked: the stream ends when the session is gone, the user
    was deleted or an admin was demoted.
  * A `:ping` comment is sent every 25 s. The stream sends `X-Accel-Buffering: no`.
* **SMTP password.** It is stored in `app.db` in plain text, because it must be usable, and it
  is never returned or logged. `PUT /settings` without `smtp.password` keeps it, and `""` clears
  it.

## Tests

`cargo test -p freelib-server` runs unit tests plus `tests/api.rs` and `tests/security.rs`
(regression tests of the security review: CSP and file headers, `inline`, DNS rebinding, login
throttling, mail recipients and limits, job caps, user deletion, folder exports, Calibre inputs;
zip bombs and forged zip sizes are generated at runtime with `freelib_import::testutil`). Each test starts the router over
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
