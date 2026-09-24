# freeLib Web — HTTP API (v1)

Source of truth for `server/crates/server` and `web/`. JSON, UTF-8, camelCase keys.
All endpoints are under `/api/v1`. Errors: HTTP status + `{"error": "<code>", "message": "<human text>"}`
(`unauthorized`, `forbidden`, `not_found`, `bad_request`, `conflict`, `unsupported_format`, `internal`).

Auth: cookie `freelib_session` (HttpOnly, SameSite=Lax). In open mode every request acts as an admin
named `admin`. `admin`-only endpoints are marked **(admin)**; others need any logged-in user.
Responses carry `ETag`; list endpoints called with `?v=<catalogVersion>` get
`Cache-Control: public, max-age=31536000, immutable`. All responses are compressed (br/gzip) when the client allows.

## Types

```ts
type LibraryStatus = { state: "idle" | "importing" | "error"; progress?: number /*0..1*/; message?: string };
type Library = {
  id: number; name: string; path: string; inpx: string | null;
  firstAuthorOnly: boolean; skipDeleted: boolean; isDefault: boolean;
  bookCount: number; authorCount: number; seriesCount: number;
  importedAt: string | null;       // RFC3339
  catalogVersion: number;          // 0 when never imported
  newSinceLastVisit: number;       // books with date > user's last visit
  status: LibraryStatus;
  opdsUrl: string;                 // absolute path, e.g. "/opds/1"
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
};
type BookDetail = Book & {
  annotation: string | null;       // sanitized HTML: <p>, <em>, <strong>, <br> only
  hasCover: boolean;
  file: string;                    // "<archive> / <file>.<ext>"
  keywords: string;
  formats: string[];               // formats this server can produce for the book, e.g. ["original","epub","kepub","azw3"]
};
type Genre = { id: number; name: string; parent: number /*0 = top*/; count: number };
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
| `GET /session` | – | `{ user: {id, username, role} \| null, openMode: boolean }` (never 401) |
| `POST /login` | `{username, password}` | `{user}` + cookie; 401 on bad credentials |
| `POST /logout` | – | 204 |

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
| `GET /libraries/:lib/authors` | `v?` | `{ version, columns: ["id","name","count"], rows: [[1,"Стругацкий Аркадий Натанович",12], …], letters: [["А", count, firstRowIndex], …] }` sorted by sort key |
| `GET /libraries/:lib/series` | `v?` | same shape as authors |
| `GET /libraries/:lib/genres` | `v?` | `Genre[]` (all 322 genres, with counts for this library; zero-count leaves included) |
| `GET /libraries/:lib/books` | exactly one of `author`, `series`, `genre`, `shelf`, `since` (YYYY-MM-DD) plus optional `lang`, `ext`, `deleted=1`, `cursor`, `limit` (default 2000, max 5000) | `{ books: Book[], nextCursor: string \| null, total: number }`. Order: author → series name, serno, title; series → serno, title; genre/shelf/since → date desc, title |
| `GET /libraries/:lib/books/:id` | – | `BookDetail` (first call may take up to ~150 ms, then cached) |
| `GET /libraries/:lib/books/:id/cover` | `size=thumb\|full` | image (`image/webp` or original jpeg/png), 404 when none. `thumb` = 240 px high |
| `GET /libraries/:lib/books/:id/file` | `format` (default `original`), `device?` (device id → its options & file name) , `inline=1` for the web reader | the file with `Content-Disposition`; 501 `unsupported_format` if the format needs Calibre and it is missing |
| `GET /libraries/:lib/search` | `q` (≥ 2 chars), `kind=all\|books\|authors\|series`, `genre` (comma ids), `lang` (comma), `ext`, `from`, `to` (YYYY-MM-DD), `limit` (books, default 200, max 1000) | `{ tookMs, authors: [{id,name,count}] (≤ 20), series: [{id,name,count,authors: string}] (≤ 20), books: Book[], total: number, facets: { genre: [[id,count]], lang: [[code,count]], ext: [[ext,count]] } }`. `q` is prefix-matched per word (FTS5 `word*`); authors/series match on `sort_key` prefix of any word |
| `GET /languages` | `lib` | `[[code, count]]` for that library |
| `PUT /libraries/:lib/books/:id/rating` | `{rating: 0..5}` | 204 |

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
| `POST /devices` | `Device` without `id` | `Device` (`shared: true` needs admin) |
| `PUT /devices/:id` | `Device` | `Device` |
| `DELETE /devices/:id` | – | 204 |
| `POST /send` | `{library, books: number[], device: number, target?: string, fileName?: string}` | `Job`. kind `send` for email, `export` for folder, `download` for download (result: single file or zip, see `downloadUrl`) |
| `GET /fonts` | – | `string[]` font family names available for embedding |

## Jobs and events

| Method & path | Response |
|---|---|
| `GET /jobs` | `Job[]` of the current user (admins also see imports), newest first, last 50 |
| `POST /jobs/:id/cancel` | `Job` |
| `DELETE /jobs?finished=1` | 204 (clears finished/failed/cancelled) |
| `GET /jobs/:id/download` | the produced file (kept 24 h) |
| `GET /events` | `text/event-stream`. Events: `event: job` data `Job`; `event: library` data `Library` (status/count changes). Heartbeat comment every 25 s |

## Settings and users

| Method & path | Body | Response |
|---|---|---|
| `GET /settings` **(admin)** | – | `{ smtp: {host, port, security: "none"\|"starttls"\|"tls", username, from, passwordSet: boolean, pauseSeconds}, opds: {enabled: boolean, requireAuth: boolean}, calibre: {available: boolean, version: string\|null} }` |
| `PUT /settings` **(admin)** | same shape; `smtp.password` write-only (omit to keep) | same as GET |
| `POST /settings/smtp/test` **(admin)** | `{to}` | 204 or 400 with message |
| `GET /users` **(admin)** | – | `[{id, username, role}]` |
| `POST /users` **(admin)** | `{username, password, role}` | user |
| `PATCH /users/:id` **(admin)** | `{password?, role?}` | user |
| `DELETE /users/:id` **(admin)** | – | 204 |
| `GET /me/prefs`, `PUT /me/prefs` | arbitrary JSON ≤ 64 KB (UI state: columns, view mode, last library/author) | JSON |

## OPDS (not under /api)

OPDS 1.2 Atom feeds, compatible with KOReader, KyBook, Moon+ Reader, FBReader, Apple Books (via apps).
Basic auth when `opds.requireAuth` (same users). Paths:

- `/opds` → navigation: libraries (or the default library directly when only one)
- `/opds/:lib` → New, Authors, Series, Genres, Search (OpenSearch `/opds/:lib/opensearch.xml`, `/opds/:lib/search?q=`)
- `/opds/:lib/authors[/:prefix]` → drill-down by prefix (letters, then 2-3 letter prefixes while > 100 entries)
- `/opds/:lib/author/:id`, `/opds/:lib/series[/:prefix]`, `/opds/:lib/series/:id`, `/opds/:lib/genres[/:id]`, `/opds/:lib/new`
- Acquisition links: `/opds/:lib/book/:id/:format` for `original`, `epub`, `kepub`, and `azw3` when Calibre is present; covers `…/cover`, thumbnails `…/cover?size=thumb`
- Legacy Qt paths `/opds_<lib>/…` redirect (301) to the new ones for the root, authors, series, genres, search.
- Pagination: 100 entries per page with `rel="next"`.

## Web app

`GET /` and any non-`/api`, non-`/opds` path → SPA `index.html` (history routing). Static assets under `/assets/*` with long cache.
SPA routes: `/`, `/l/:lib/new`, `/l/:lib/authors[/:id]`, `/l/:lib/series[/:id]`, `/l/:lib/genres[/:id]`, `/l/:lib/shelves/:id`,
`/l/:lib/search?q=…`, `/l/:lib/book/:id` (phone), `/l/:lib/read/:id`, `/libraries`, `/settings[/:section]`, `/login`.
