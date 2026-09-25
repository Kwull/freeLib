# freeLib Web — HTTP API (v1)

Source of truth for `server/crates/server` and `web/`. JSON, UTF-8, camelCase keys.
All endpoints are under `/api/v1`. Errors: HTTP status + `{"error": "<code>", "message": "<human text>"}`
(`unauthorized`, `forbidden`, `not_found`, `bad_request`, `conflict`, `unsupported_format`, `rate_limited`, `stale`, `internal`).
`stale` (503) means the library was re-imported during the request; retry it.

Security headers: every response carries `X-Content-Type-Options: nosniff` and, except book files and covers,
`Content-Security-Policy: default-src 'self'; script-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline' blob:; font-src 'self' data: blob:; connect-src 'self'; frame-src 'self' blob:; worker-src 'self'; form-action 'self'`.
Book files (`…/file`, job downloads, OPDS acquisitions) and covers get `Content-Security-Policy: sandbox`.
In open mode (and whenever `FREELIB_ALLOWED_HOSTS` is set) requests whose `Host` is not `localhost`, an IP literal
or an allowed name are answered with 421 (DNS rebinding protection).

Auth: cookie `freelib_session` (HttpOnly, SameSite=Lax). In open mode every request acts as an admin
named `admin`. `admin`-only endpoints are marked **(admin)**; others need any logged-in user.
Responses carry `ETag`; list endpoints called with `?v=<catalogVersion>` get
`Cache-Control: public, max-age=31536000, immutable`. All responses are compressed (br/gzip) when the client allows.

## Types

```ts
type LibraryStatus = {
  state: "idle" | "importing" | "error"; progress?: number /*0..1*/; message?: string /*current step, or the error*/;
  // set when the server started the import by itself: "upgrade" = the catalog was written by another
  // schema version and is rebuilt after an update; "autoimport" = FREELIB_AUTOIMPORT
  reason?: "upgrade" | "autoimport";
};
// A library is browsable when importedAt is set. While a library without a catalog imports (first import,
// rebuild after an update), its browsing endpoints answer 404 "library is not imported yet"; a re-import of a
// browsable library keeps serving the old catalog until the new one is swapped in.
type Library = {
  id: number; name: string; path: string; inpx: string | null;
  firstAuthorOnly: boolean; skipDeleted: boolean; isDefault: boolean;
  bookCount: number; authorCount: number; seriesCount: number;
  importedAt: string | null;       // RFC3339
  catalogVersion: number;          // 0 when never imported
  newSinceLastVisit: number;       // books dated on/after the server-local day of the previous visit
  status: LibraryStatus;
  opdsUrl: string;                 // absolute path, e.g. "/opds/1"
  externalRatings: { lookedUp: number; found: number; rated: number };  // Open Library lookups of this library
};
type AuthorRef = { id: number; name: string };
type SeriesRef = { id: number; name: string };
type Book = {
  id: number; key: string;         // key = book_key
  title: string;
  authors: AuthorRef[];
  series: SeriesRef | null; serno: number | null;
  genres: number[];                // genre ids
  lang: string; ext: string; size: number;
  date: string;                    // YYYY-MM-DD
  deleted: boolean;
  rating: number;                  // user rating 0..5 (0 = none)
  shelves: number[];               // shelf ids of the current user
  libRating: number;               // library rating: INPX LIBRATE/STARS, 0..5 (0 = none)
  extRating: { avg: number; votes: number } | null;  // Open Library average + votes (cached; null = unknown or no votes)
  kidsAge: 0 | 6 | 12 | 16 | 18 | null;  // age estimate (heuristic from genres and keywords, see ARCHITECTURE.md); null = unknown
};
type BookDetail = Book & {
  annotation: string | null;       // sanitized HTML: <p>, <em>, <strong>, <br> only
  hasCover: boolean;
  file: string;                    // "<archive> / <file>.<ext>"
  keywords: string;
  formats: string[];               // formats this server can produce for the book, e.g. ["original","epub","kepub","azw3"]
  extRatingInfo: {                 // the cached Open Library lookup, null = not looked up yet (opening the book queues it)
    source: "openlibrary"; status: "found" | "not_found" | "error";
    average: number | null; count: number; workKey: string | null; url: string | null; fetchedAt: string;
  } | null;
};
type Genre = { id: number; name: string; parent: number /*0 = top*/; count: number };
type AuthorSummary = {             // GET /libraries/:lib/authors/:id/summary; live books only
  id: number; name: string;
  count: number;                   // books
  anthologies: number;             // books with ≥ 4 authors (anthologies / collections)
  series: { id: number; name: string; count: number }[];  // series of the author's books, most books first
  withoutSeries: number;
  langs: [string, number][];       // most frequent first
  genres: [number, number][];      // top 8 assigned genres
  firstDate: string; lastDate: string;  // oldest / newest `date`, "" when unknown
  coauthors: { id: number; name: string; books: number; direct: number }[];
                                   // ≤ 10 co-authors with ≥ 1 direct or ≥ 2 shared books, ranked by
                                   // direct desc, books desc, name. books = shared books, direct = shared
                                   // books with < 4 authors (real co-authorship, not an anthology)
  coauthorCount: number;           // everybody sharing a book
};
type Shelf = { id: number; name: string; color: string /*#rrggbb*/; count: number };
type Device = {
  id: number; name: string;
  kind: "email" | "download" | "folder";
  format: "original" | "epub" | "kepub" | "azw3" | "mobi" | "pdf";
  target: string | null;           // email address (email) or sub-folder under FREELIB_EXPORT_DIR (folder)
  fileName: string;                // template, see below
  shared: boolean;                 // true = visible to all users (admin-created)
  options: ConvertOptions;
};
type ConvertOptions = {
  hyphenate: "none" | "soft" | "full";
  footnotes: "end" | "inline" | "popup";
  dropCaps: boolean;
  breakAfterChapter: boolean;
  tocPlacement: "start" | "end" | "none";
  createCover: "never" | "missing" | "always";
  coverLabel: string | null;       // template drawn on generated covers, e.g. "%s %n"
  joinSeries: boolean;             // several books of one series -> one file
  transliterate: boolean;          // transliterate file names
  annotation: boolean;             // put annotation page after the cover
  fontFamily: string | null;       // one of GET /fonts names, null = reader default
  userCss: string | null;
};
type Job = {
  id: string; kind: "import" | "send" | "export" | "download";
  title: string;                   // e.g. "Send to Kindle · 3 books"
  state: "queued" | "running" | "done" | "failed" | "cancelled";
  progress: number;                // 0..1
  message: string;                 // current step or error
  log: string[];                   // last ≤ 50 lines (import)
  downloadUrl: string | null;      // for finished download/export jobs
  createdAt: string; finishedAt: string | null;
};
```

File name template placeholders (same as the Qt app): `%a` author (first author "Last F."), `%fa` full author name,
`%s` series, `%n` number in series (2 digits), `%b` title, `%l` language, `%y` year of the date field. Missing parts collapse with
their separator. Result is sanitised for file systems; `transliterate` applies Russian → Latin.

## Session

| Method & path | Body | Response |
|---|---|---|
| `GET /session` | – | `{ user: {id, username, role} \| null, openMode: boolean, auth: { password: boolean, oidc: { enabled: true, label: string } \| null } }` (never 401). `auth.password` is false with `FREELIB_OIDC_DISABLE_PASSWORD`; `auth.oidc.label` is the sign-in button text (`FREELIB_OIDC_BUTTON`) |
| `POST /login` | `{username, password}` | `{user}` + cookie (a new session id; a session cookie the browser already had is invalidated); 401 on bad credentials; 403 when password sign-in is disabled (`FREELIB_OIDC_DISABLE_PASSWORD`; the `FREELIB_ADMIN_USER` account keeps it while `FREELIB_ADMIN_PASSWORD` is set); 429 `rate_limited` after 5 failures per client address (IPv6: per /64) **or** per user name (the wait doubles with each further failure, max 5 min; attempts in flight count) |
| `POST /logout` | – | 204 |

### Single sign-on (OpenID Connect)

Configured with `FREELIB_PUBLIC_URL` and `FREELIB_OIDC_*` (DOCKER.md). All endpoints answer 404 when it is not
configured. The flow is the authorization code flow with PKCE (S256), `state` and `nonce`; see ARCHITECTURE.md
"Single sign-on" for the validation rules.

| Method & path | Body / query | Response |
|---|---|---|
| `GET /auth/oidc/login` | `return=/path` (optional; only same-origin relative paths, anything else becomes `/`) | 303 to the provider's authorization endpoint, with an HttpOnly `freelib_oidc` state cookie (`Path=/api/v1/auth/oidc`, `SameSite=Lax`, 10 min, `Secure` when `FREELIB_PUBLIC_URL` is https or `X-Forwarded-Proto: https`). A full page navigation, not a fetch. When the provider cannot be reached: 303 to `/login?ssoError=unavailable` |
| `GET /auth/oidc/callback` | `code`, `state` (or `error`, `error_description`) from the provider | success: 303 to the return path with a new session cookie (the old session, if any, is invalidated). Failure: 303 to `/login?ssoError=<code>` (link flow: `/settings/account?ssoError=<code>`). Codes: `state` (unknown, expired, used, or not this browser's sign-in), `provider` (the provider returned an error, e.g. the user cancelled), `token` (code exchange or ID token validation failed; details in the server log), `unavailable`, `not_linked` (no account and `FREELIB_OIDC_AUTO_CREATE=false`), `already_linked`, `session` (link flow finished in a browser not signed in as that user), `rate_limited` (failed callbacks count against the login rate limit per address), `busy` |
| `POST /auth/oidc/link` | – (signed in) | `{ url }`: the web app navigates there to link the provider identity to the current account; the callback returns to `/settings/account?sso=linked` with a new session id |
| `GET /me/account` | – | `{ user, hasPassword: boolean, passwordLogin: boolean, sso: { label, linked: boolean, email: string \| null, lastLogin: string \| null } \| null }` (`sso` is null without single sign-on) |
| `PUT /me/password` | `{password, current?}` | 204 + a new session cookie; `current` is required (403 when wrong, rate limited like `/login`) when the account has a password. Accounts created by single sign-on have none: this sets one for OPDS apps. Other sessions of the user are signed out |
| `DELETE /me/oidc` | – | 204; 409 when the account has no password or password sign-in is disabled (it would lock the user out); 404 when nothing is linked |

Accounts are matched by (issuer, `sub`) only, never by user name or e-mail. OPDS keeps HTTP Basic auth with local
passwords: a user created by single sign-on sets a password in Settings → Account to use OPDS apps.

## Libraries

| Method & path | Body / query | Response |
|---|---|---|
| `GET /libraries` | – | `Library[]` |
| `POST /libraries` **(admin)** | `{name, path, inpx?, firstAuthorOnly?, skipDeleted?, isDefault?}`; `path` and `inpx` must be inside `FREELIB_BOOKS_DIR` | `Library` (import job started automatically when `inpx` given) |
| `PATCH /libraries/:lib` **(admin)** | any subset of POST fields | `Library` |
| `DELETE /libraries/:lib` **(admin)** | – | 204 (removes `lib_<id>.db` and cache) |
| `POST /libraries/:lib/import` **(admin)** | `{mode: "full" \| "new"}` | `Job` (409 if an import is already running) |
| `GET /fs` **(admin)** | `?path=<dir under books dir>` | `{path, parent: string\|null, entries: [{name, dir: boolean, size}]}` (dirs, `.inpx`, `.zip` only) |

## Browsing (per library; `:lib` = library id)

| Method & path | Query | Response |
|---|---|---|
| `GET /libraries/:lib/authors` | `v?` | `{ version, columns: ["id","name","count"], rows: [[1,"Стругацкий Аркадий Натанович",12], …], letters: [["А", count, firstRowIndex], …] }`. Only names with at least one live (non-deleted) book. Sorted by sort key (accented Latin letters fold to their base letter: `Čapek` sorts and indexes under `C`, see `normalize-vectors.json`), except that names not starting with a letter (digits, symbols) form a `#` group **at the end**. `letters` are in row order, one entry per letter |
| `GET /libraries/:lib/series` | `v?` | same shape as authors |
| `GET /libraries/:lib/authors/:id/summary` | – | `AuthorSummary` (below); 404 for an unknown author |
| `GET /libraries/:lib/authors/:id/coauthors` | – | `{ columns: ["id","name","books","direct"], rows: [[id, name, books, direct], …] }`: everybody sharing a live book with the author, ranked like `AuthorSummary.coauthors` (not filtered); 404 for an unknown author |
| `GET /libraries/:lib/genres` | `v?`, `lang=en\|ru\|uk` (default `en`) | `Genre[]` (all 322 genres, with counts for this library; zero-count leaves included). `name` is localized to `lang`; the ETag includes `lang`, so switching the SPA's language triggers a refetch |
| `GET /libraries/:lib/books` | exactly one of `author`, `series`, `genre`, `shelf`, `since` (YYYY-MM-DD) plus optional `lang`, `ext`, `deleted=1`, `q` (text filter: every word is a prefix of a word of the title, authors, series or keywords, like search), `cursor`, `limit` (default 2000, max 5000) | `{ books: Book[], nextCursor: string \| null, total: number }`. Order: author → series name, serno, title; series → serno, title; genre/shelf/since → date desc, title |
| `GET /libraries/:lib/books/:id` | – | `BookDetail` (first call may take up to ~150 ms, then cached) |
| `GET /libraries/:lib/books/:id/cover` | `size=thumb\|full` | image (`image/webp` or original jpeg/png); `full` is 404 when the book has no cover. `thumb` = 240 px high; when the book has no cover, a generated SVG placeholder tile (background colour from the title, author + title text, like the SPA's own placeholder) is returned instead of 404, with header `X-Cover: generated` |
| `GET /libraries/:lib/books/:id/file` | `format` (default `original`), `device?` (device id → its options & file name) , `inline=1` for the web reader | the file with `Content-Disposition: attachment`; `inline=1` is honoured for EPUB only. HTML, XHTML, XML, FB2 and SVG files are sent as `application/octet-stream`. Originals are streamed (no `Content-Length` for deflated zip entries); books above 256 MiB → 413. 501 `unsupported_format` if the format needs Calibre and it is missing, or the book is neither FB2 nor EPUB (other formats are offered as `original` only) |
| `GET /libraries/:lib/search` | `q` (≥ 2 chars), `kind=all\|books\|authors\|series`, `genre` (comma ids), `lang` (comma), `ext`, `from`, `to` (YYYY-MM-DD), `limit` (books, default 200, max 1000) | `{ tookMs, authors: [{id,name,count}] (≤ 20), series: [{id,name,count,authors: string}] (≤ 20), books: Book[], total: number, facets: { genre: [[id,count]], lang: [[code,count]], ext: [[ext,count]] } }`. `q` is prefix-matched per word (FTS5 `word*`); authors/series match on `sort_key` prefix of any word |
| `GET /languages` | `lib` | `[[code, count]]` for that library |
| `PUT /libraries/:lib/books/:id/rating` | `{rating: 0..5}` | 204 |

### Rating filters and sorts

`GET books` and `GET search` also take (all optional; invalid values → 400):

| Parameter | Meaning |
|---|---|
| `sort=my\|lib\|ext` | sort by my rating / library rating / Open Library average (votes break ties), best first, **unrated last**; equal ratings keep the list's own order (search: relevance). Other values keep the normal order |
| `minMy=1..5`, `minLib=1..5` | minimum own / library rating |
| `minExt=0..5` (decimal), `minExtVotes=N` | minimum Open Library average / vote count (books without an Open Library rating are excluded) |
| `unratedByMe=1` | only books the user has not rated |
| `kidsMaxAge=N` | only books whose age estimate is known and ≤ N |

With any of them the whole selection is filtered and sorted on the server, in memory (genre and new-arrival
selections from the per-book attribute table, author/series/shelf ids from one query), then paged by offset;
genre / `since` lists are ordered date desc, id (instead of date desc, title) before a rating sort. `total` counts the
filtered books; search facets count the books left after the rating filters. Books of an author or series listed
on a first page are queued for an Open Library lookup (priority 2).

## Shelves

| Method & path | Body | Response |
|---|---|---|
| `GET /shelves` | – | `Shelf[]` (current user) |
| `POST /shelves` | `{name, color}` | `Shelf` |
| `PATCH /shelves/:id` | `{name?, color?}` | `Shelf` |
| `DELETE /shelves/:id` | – | 204 |
| `POST /shelves/:id/books` | `{library, books: number[], add: boolean}` | `Shelf` |

Books of a shelf: `GET /libraries/:lib/books?shelf=:id`.

## Devices and sending

| Method & path | Body | Response |
|---|---|---|
| `GET /devices` | – | `Device[]` (shared + own). A fresh install has shared defaults: "Kindle" (email, epub), "Kindle (USB)" (download, azw3), "Apple Books" (download, epub), "Kobo" (download, kepub), "Server folder" (folder, epub), "Original" (download, original) |
| `POST /devices` | `Device` without `id` | `Device` (`shared: true` and `kind: "folder"` need admin; an `email` target must match `smtp.allowedRecipients`, else 403) |
| `PUT /devices/:id` | `Device` | `Device` (shared and folder devices: admin only) |
| `DELETE /devices/:id` | – | 204 |
| `PUT /devices/order` | `{ids: number[]}` | `Device[]` in the new order. Per user: `ids` first (any subset of the devices the user sees, shared ones included), the others after them. `GET /devices` returns the user's order (devices never ordered follow, oldest first). **The first device is the user's default** (the quick-send button, the Send dialog's preselection, MCP `device: "default"`). 404 for an unknown id, 400 for duplicates |
| `POST /send` | `{library, books: number[], device: number, target?: string, fileName?: string, options?: Partial<ConvertOptions>}` | `Job`. kind `send` for email, `export` for folder, `download` for download (result: single file or zip, see `downloadUrl`). `options` is merged (shallow) over the device's own options for this send only; the device itself is not changed. E-mail: the recipient must match `smtp.allowedRecipients` (403 `forbidden` otherwise, admins included) and the user's mails today plus this request's books must not exceed `smtp.dailyLimitPerUser` (429 `rate_limited`). Folder: readers may only use shared folder devices with their configured target (403). At most 5 queued + running send/export/download jobs per user (429 `rate_limited`). Exports never overwrite: an existing file gets a ` (2)`, ` (3)`, … sibling |
| `GET /fonts` | – | `string[]` font family names available for embedding |

## Jobs and events

| Method & path | Response |
|---|---|
| `GET /jobs` | `Job[]` of the current user (admins also see imports), newest first, last 50 |
| `POST /jobs/:id/cancel` | `Job` (a running Calibre conversion is killed) |
| `DELETE /jobs?finished=1` | 204 (clears finished/failed/cancelled) |
| `GET /jobs/:id/download` | the produced file (kept 24 h) |
| `GET /events` | `text/event-stream`. Events: `event: job` data `Job`; `event: library` data `Library` (status/count changes). Heartbeat comment every 25 s. The stream ends when the session ends, the user is deleted or an admin is demoted (reconnect to continue) |

## Settings and users

| Method & path | Body | Response |
|---|---|---|
| `GET /settings` **(admin)** | – | `{ externalRatings: {enabled, source: "openlibrary", contactSet, progress: {lookedUp, found, rated, total}, queued, requests, pausedFor (s), lastError}, mcp: {enabled, url}, smtp: {host, port, security: "none"\|"starttls"\|"tls", username, from, passwordSet: boolean, pauseSeconds, allowedRecipients: string[], dailyLimitPerUser: number, subject: string (mail subject template: `%b` title, `%a` author; default `%b`)}, opds: {enabled: boolean, requireAuth: boolean}, calibre: {available: boolean, version: string\|null} }` |
| `PUT /settings` **(admin)** | same shape (`externalRatings.enabled`, default true: when false the server sends nothing to Open Library; `mcp.enabled`, default true: when false `/mcp` answers 403; the other `externalRatings` fields are read-only); `smtp.password` write-only (omit to keep, `""` to remove). `allowedRecipients`: patterns where `*` matches any characters, compared case-insensitively with the whole address (default `["*@kindle.com", "*@free.kindle.com"]`; a lone `*` allows every address; at most 100, each `*` or containing `@`, else 400). `dailyLimitPerUser`: mails per user and server-local day (default 100) | same as GET |
| `POST /settings/smtp/test` **(admin)** | `{to}` | 204 or 400 with message |
| `GET /users` **(admin)** | – | `[{id, username, role, hasPassword: boolean, sso: {issuer, email, createdAt, lastLogin} \| null}]` (`sso`: the linked single sign-on identity) |
| `POST /users` **(admin)** | `{username, password, role}` | user; 409 when the name exists (case-insensitive). User ids are never reused |
| `PATCH /users/:id` **(admin)** | `{password?, role?}` | user |
| `DELETE /users/:id` **(admin)** | – | 204 (also cancels and removes the user's jobs and their files) |
| `GET /me/tokens` | – | `{ tokens: ApiToken[], scopes: ["read","write","send"], mcp: {enabled, url} }` (`url`: `FREELIB_PUBLIC_URL` + `/mcp`, else built from the request's host) |
| `POST /me/tokens` | `{name, scopes: ("read"\|"write"\|"send")[], expiresInDays?: 1..3650}` | `{ token: ApiToken, secret: "fl_…" }` — the secret is returned **only here**; the server keeps its SHA-256. At most 50 tokens per user (409) |
| `DELETE /me/tokens/:id` | – | 204 (revoked at once, also for cached authentications); 404 for another user's token |
| `GET /me/tokens/audit` | – | the last 50 MCP tool calls with the user's tokens: `[{id, tokenId, tokenName (null when revoked), tool, ok, detail (arguments, ≤ 200 chars), at}]` newest first |
| `GET /me/prefs`, `PUT /me/prefs` | arbitrary JSON ≤ 64 KB (UI state: pane and column widths, visible columns, sort orders, view mode, last library and device…; the SPA writes the whole object) | JSON |

`ApiToken = { id, name, prefix /* "fl_" + 8 chars */, scopes, createdAt, lastUsedAt /* updated ≤ once a minute */, expiresAt: string | null }`.
Tokens are accepted by `/mcp` only (not by the REST API or OPDS); the REST endpoints above need the session cookie.

## MCP (not under /api)

`POST /mcp` — Model Context Protocol, streamable HTTP transport (official Rust SDK `rmcp`), **stateless**: no
`Mcp-Session-Id`; each POST carries one JSON-RPC message and is answered with `application/json` (or SSE when a tool
streams notifications). Protocol versions up to `2026-07-28`. Requires `Authorization: Bearer fl_…` (401 with a
`WWW-Authenticate: Bearer` challenge otherwise), 403 when `mcp.enabled` is false, 429 + `Retry-After` above
`FREELIB_MCP_RATE` requests per token and minute (default 120). `tools/list` lists only the tools the token's scopes
allow; every `tools/call` is checked again (a refused call is a tool error naming the missing scope) and audited.

| Tool | Scope | Arguments (all ids are per library; `library` optional, default library otherwise) |
|---|---|---|
| `list_libraries` | read | – |
| `search_books` | read | `query`, `author`/`author_id`, `series`/`series_id`, `genre`/`genre_id`, `language`, `added_after`, `added_before`, `min_my_rating`, `min_library_rating`, `min_openlibrary_rating`, `min_openlibrary_votes`, `unrated_by_me`, `kids_max_age`, `sort` (`relevance`\|`date`\|`my_rating`\|`library_rating`\|`openlibrary_rating`), `limit` ≤ 50, `cursor` |
| `get_book` | read | `id` → authors, series + number, genres, language, added, size, format, formats, annotation (plain text ≤ 4000 chars), keywords, my / library / Open Library rating, kids age, my shelves, `myHistory {lastSent, lastDownloaded, lastRead}` |
| `get_author` | read | `id` or `name` → counts, series, genres, languages, years, co-authors, best-rated books |
| `list_author_books` | read | `author_id`, `sort`, `limit` ≤ 100, `cursor` |
| `get_series` | read | `id` or `name` → books in order with my actions, `nextUnread` |
| `list_genres` | read | `parent`, `lang` |
| `get_reading_profile` | read | – → shelves with books, ratings, recent sends/downloads/reads, top genres and authors |
| `suggest_candidates` | read | `seed_book_ids`, `use_profile`, `genres`, `language`, `kids_max_age`, `min_library_rating`, `min_openlibrary_rating`, `exclude_read` (default true), `limit` ≤ 50 → scored candidates with `reasons` |
| `get_external_rating` | read | `id` → cached, or looked up now (respecting the rate limit; 403 when disabled) |
| `list_devices` | read | – → devices in the user's order, `default` = the first |
| `list_shelves` | read | – |
| `add_to_shelf`, `remove_from_shelf` | write | `shelf_id` or `shelf` (name; `create: true` makes it), `book_ids` ≤ 500 |
| `rate_book` | write | `id`, `rating` 0..5 |
| `send_books` | send | `book_ids` ≤ 50, `device` (id or `"default"`) → `{jobId}`; same rules as `POST /send` (allowed recipients, daily limit, 5 jobs per user) |
| `get_job` | send | `id` |

Prompts: `suggest_next_book` (`wishes?`), `books_for_kid` (`age`, `interests?`), `similar_to` (`book_id`).

## OPDS (not under /api)

OPDS 1.2 Atom feeds, compatible with KOReader, KyBook, Moon+ Reader, FBReader, Apple Books (via apps).
Basic auth when `opds.requireAuth` (same users and the same login rate limits). Paths:

- `/opds` → navigation: libraries (or the default library directly when only one)
- `/opds/:lib` → New, Authors, Series, Genres, Search (OpenSearch `/opds/:lib/opensearch.xml`, `/opds/:lib/search?q=`)
- `/opds/:lib/authors[/:prefix]` → drill-down by prefix (letters, then 2-3 letter prefixes while > 100 entries)
- `/opds/:lib/author/:id`, `/opds/:lib/series[/:prefix]`, `/opds/:lib/series/:id`, `/opds/:lib/genres[/:id]`, `/opds/:lib/new`
- Acquisition links: `/opds/:lib/book/:id/:format` for `original`, `epub`, `kepub`, and `azw3` when Calibre is present; covers `…/cover`, thumbnails `…/cover?size=thumb`
- Legacy Qt paths `/opds_<lib>/…` redirect (301) to the new ones for the root, authors, series, genres, search.
- Genre names are localized from the request's `Accept-Language` (`ru` or `uk` recognized, highest `q` wins); anything else, including no header, is served in English.
- Pagination: 100 entries per page with `rel="next"`.

## Web app

`GET /` and any non-`/api`, non-`/opds` path → SPA `index.html` (history routing). Static assets under `/assets/*` with long cache.
SPA routes: `/`, `/l/:lib/new`, `/l/:lib/authors[/:id][?book=:id]`, `/l/:lib/series[/:id][?book=:id]`, `/l/:lib/genres[/:id]`, `/l/:lib/shelves/:id`,
`/l/:lib/search?q=…`, `/l/:lib/book/:id` (phone), `/l/:lib/read/:id`, `/libraries`, `/settings[/:section]`, `/login`.
