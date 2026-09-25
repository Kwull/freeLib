# freelib-catalog

Catalog database (`lib_<id>.db`) schema and **read API**, `app.db` schema + migrations,
sort-key normalisation and the static genre table. Everything is synchronous (rusqlite);
call it from `tokio::task::spawn_blocking`. Result types are `serde::Serialize` with camelCase
keys; user-specific fields (`rating`, `shelves`, annotation, cover, formats) are added by the server.

The importer lives in `freelib-import` (see the end of this file).

## Opening catalogs

```rust
// one per library, e.g. in AppState: HashMap<LibId, CatalogHandle>
let handle = CatalogHandle::new(data_dir.join(format!("lib_{id}.db"))); // opens if the file exists
let cat: Option<Arc<Catalog>> = handle.get();   // None = never imported / unusable
handle.reload()?;                               // after an import renamed the new file into place
handle.close();                                 // before deleting the library
```

* Take `handle.get()` **once per request** and use that `Arc<Catalog>` throughout, so a reload never
  mixes two catalog versions in one response. Old `Catalog`s keep working until dropped.
* `Catalog::open(path) -> Result<Catalog>` fails with `CatalogError::SchemaVersion` when the file was
  written by another schema version (→ re-import) and `CatalogError::NotFound` when missing.
* Errors (`CatalogError`): `Sqlite`, `SchemaVersion` (re-import), `NotFound`, `BadCursor` (→ 400), `Stale` — an old
  `Catalog` needed a new connection after the file was replaced; take `handle.get()` again and retry.
* Connections: read-only (`SQLITE_OPEN_READ_ONLY`, `query_only`, `mmap_size=1 GiB`, `cache_size=-65536`,
  `temp_store=MEMORY`), pooled (≤ 8 idle, more opened on demand), statements cached.
* Search, `count_newer_than` and large genre/since counts use `Catalog::attrs()`: a ~9 MB
  per-book attribute table built on first use (~250 ms for 600k books). After `reload()`, warm it in the
  background: `spawn_blocking(move || cat.attrs())`.

## Queries (`impl Catalog`)

| Method | Returns | API |
|---|---|---|
| `stats() -> &LibraryStats` | `bookCount` (all), `liveBookCount`, `authorCount`, `seriesCount`, `importedAt`, `catalogVersion`, `inpxVersion`, `sourceInpx`, `collectionName`, `firstAuthorOnly`, `skipDeleted` (cached at open) | `Library` |
| `catalog_version() -> i64` | changes on every import (ms timestamp, strictly increasing) | `?v=`, ETag |
| `authors() -> NameList` | `{rows: [(id, name, count)], letters: [(letter, count, firstRowIndex)]}` sorted by `sort_key, id` | `GET authors` |
| `series_list() -> NameList` | same for series | `GET series` |
| `author(id) -> Option<NameCount>` | `{id, name, count}` | page titles |
| `series(id) -> Option<SeriesHit>` | `{id, name, count, authors}` (`authors` = main author(s), display string) | |
| `genres(lang) -> Vec<GenreCount>` | all 322 `{id, name, parent, count}`, `name` localized to `lang` ("en"/"ru"/"uk"); group count = distinct books in the group | `GET genres` |
| `languages() -> Vec<(String, i64)>` | `[(lang, count)]`, most frequent first | `GET /languages` |
| `count_newer_than(date_or_rfc3339) -> i64` | live books with `date > d` | `newSinceLastVisit` |
| `books(&BookSelector, &BookFilter, &Page) -> BookPage` | `{books: Vec<Book>, nextCursor, total}` | `GET books` |
| `books_by_ids(&[i64]) -> Vec<Book>` | in the given order, no filters | send/export, OPDS |
| `book(id) -> Option<BookDetail>` | `Book` + `file`, `archive`, `folder`, `keywords`, `libId`, `stars`, `archOffset`, `archCsize`, `archMethod`; helpers `entry_name()`, `relative_path()`, `display_file()` (`"<archive> / <file>.<ext>"`) | `GET books/:id` |
| `search(&SearchQuery) -> SearchResult` | `{tookMs, authors ≤20, series ≤20, books, total, facets: {genre, lang, ext}}` | `GET search` |
| `ids_by_keys(&[String]) -> Vec<(String, i64)>` | `book_key → id` for keys present | shelves, ratings |
| `keys_by_ids(&[i64]) -> Vec<(i64, String)>` | `id → book_key` | storing shelves/ratings |
| `conn() -> PooledConn` | raw read-only connection (derefs to `rusqlite::Connection`) for anything else | |

```rust
pub enum BookSelector { Author(i64), Series(i64), Genre(u16), Since(String /*YYYY-MM-DD*/), Ids(Vec<i64>) }
pub struct BookFilter { pub langs: Vec<String>, pub ext: Option<String>, pub include_deleted: bool } // Default = no filter, deleted hidden
pub struct Page { pub cursor: Option<String>, pub limit: usize }  // Default: None, 2000. Clamp limit to 5000 in the server.
pub struct SearchQuery { pub q: String, pub kind: SearchKind /*All|Books|Authors|Series, Deserialize lowercase*/,
    pub genres: Vec<u16>, pub langs: Vec<String>, pub ext: Option<String>,
    pub from: Option<String>, pub to: Option<String>, pub include_deleted: bool, pub limit: usize /*clamped 1..=1000*/ }
pub struct Book { id, key, title, authors: Vec<AuthorRef{id,name}>, series: Option<SeriesRef{id,name}>, serno: Option<i64>,
    genres: Vec<u16>, lang, ext, size: i64, date: String, deleted: bool }
```

Semantics:

* Ordering: author → series name (books without series last), serno, title; series → serno, title;
  genre / since / ids → date desc, title. Ties broken by id, so pages are stable.
* `Genre(id)` of a top-level group includes all its sub-genres.
* Cursors are opaque strings (currently the row offset); an invalid cursor → `CatalogError::BadCursor` (→ 400).
* Deleted books are hidden unless `include_deleted` (API `deleted=1`). Counts in author/series lists,
  genres and languages are **live** (non-deleted) books; an author whose books are all deleted is listed with 0.
* Search: `q` needs ≥ 2 characters; every word is a prefix (`word*`, AND-ed); one-letter words are ignored
  when longer ones exist; `ё = е`. Books are ranked by bm25 (title > authors > series > keywords), then newest.
  Authors/series match when every query word prefixes a word of the name (FTS over names), ordered by book count.
  Facets are disjunctive (each facet ignores its own filter); `facets.genre` holds leaf/assigned genre ids.
  `total` = matching books after filters (not capped by `limit`).

## Reading book bytes

`BookDetail.arch_offset` is the offset of the zip **local file header** (NULL when not resolved). Data starts at
`offset + 30 + name_len + extra_len` (u16 LE at header bytes 26 and 28; `freelib_import::zipdir::local_data_start`
does this); read `arch_csize` bytes and inflate raw deflate when `arch_method == 8` (0 = stored). If the offset is
NULL or the header signature does not match, fall back to opening the zip and looking up `entry_name()`.
Plain files (`archive == ""`) live at `library_path / relative_path()`.

## Other modules

* `normalize::normalize(&str) -> String` — sort key (lower-case, `ё→е`, quotes dropped, punctuation → space,
  collapsed whitespace, leading non-alphanumerics dropped). `letter_of(sort_key)`, `search_tokens(q)`, `collapse_ws`.
* `genres::genres() -> &'static Genres` — `all()`, `get(id)`, `by_code(code)`, `resolve(code)` (unknown codes →
  "…: прочее" of the matching group, else 11 "Прочее"), `top_level()`, `children(id)`, `with_descendants(id)`, `top(id)`.
  Each `GenreDef` has `name` (Russian), `name_en`, `name_uk`, plus `localized(lang)` ("en"/"ru"/"uk", falling back to
  English then Russian).
* `schema` — `create_catalog_tables/indexes` (importer), `CATALOG_SCHEMA_VERSION`, and for `app.db`:
  `open_app_db(path) -> Connection` (WAL, foreign keys, busy timeout, migrates), `migrate_app_db(&conn)`,
  `APP_MIGRATIONS` (append-only list; `PRAGMA user_version` = number applied), `APP_SCHEMA_VERSION`.
* `util` — `now_rfc3339()`, `now_millis()`, `parse_date()`, civil date helpers.

## Importer (`freelib-import`) in one screen

```rust
let stats: ImportStats = freelib_import::import_inpx(
    &ImportOptions { inpx, db_path /* lib_<id>.db */, library_dir: Some(lib_path), resolve_offsets: true,
                     first_author_only, skip_deleted, threads: 0 },
    &|done, total, msg| { /* job progress = done / total */ },
    &cancel_flag /* AtomicBool */,
)?;                       // builds lib_<id>.new.db, renames it over lib_<id>.db
handle.reload()?;         // then warm: cat.attrs()
```
`ImportError::Cancelled` on cancellation (the old catalog is untouched). Mode "new" = run the same full import.
Other entry points: `resolve_offsets(db, lib_dir, progress, cancel)` (post-import pass),
`migrate::read_qt_database(path) -> QtMigration` + `QtMigration::apply(&mut app_conn, user_id) -> ApplyStats`,
`synth::generate(out, &GenOptions)` (test data), `zipdir::{read_central_directory, local_data_start}`.
