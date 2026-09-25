//! MCP tools: definitions (name, scope, description, JSON schema of the arguments) and
//! implementations. Outputs are compact JSON objects with the ids needed for the next call.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use freelib_catalog::{
    Book, BookFilter, BookSelector, Catalog, Page, RatingQuery, RatingSort, SearchKind,
    SearchQuery, genres,
};
use rmcp::handler::server::common::schema_for_type;
use rmcp::model::{Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::api::browse::{BookOut, with_marks, with_rating_source};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::mcp::suggest::{self, BookFacts, Profile, Seed};
use crate::state::AppState;
use crate::tokens::TokenAuth;

type ToolResult = Result<Value, String>;

fn err(e: ApiError) -> String {
    e.message
}

/// (tool, scope) of every tool, in `tools/list` order.
pub fn definitions() -> Vec<(Tool, &'static str)> {
    fn t<P: JsonSchema + 'static>(
        name: &'static str,
        title: &str,
        desc: &'static str,
        read_only: bool,
    ) -> Tool {
        let ann = ToolAnnotations::with_title(title)
            .read_only(read_only)
            .destructive(false)
            .open_world(false);
        Tool::new(name, desc, schema_for_type::<P>()).with_annotations(ann)
    }
    vec![
        (
            t::<LibraryArg>(
                "list_libraries",
                "List libraries",
                "Libraries of this server with book counts; the default one is used when a tool gets no `library`.",
                true,
            ),
            "read",
        ),
        (
            t::<SearchArgs>(
                "search_books",
                "Search books",
                "Find books by text (title, author, series, keywords; every word is a prefix) and/or filters: author, series, genre, language, date added, minimum ratings, kids age, not rated by me. Without `query` give an author, series, genre or added_after. Sort: relevance (default with a query), date, my_rating, library_rating, openlibrary_rating. Paginated (`cursor`).",
                true,
            ),
            "read",
        ),
        (
            t::<BookArg>(
                "get_book",
                "Book details",
                "Full details of one book: authors, series and number, genres, language, date added, size and format, annotation text, my / library / Open Library ratings, kids age estimate, my shelves, and whether I sent, downloaded or read it.",
                true,
            ),
            "read",
        ),
        (
            t::<AuthorArg>(
                "get_author",
                "Author summary",
                "Summary of an author (by id or name): book count, series, genres, languages, years, top co-authors and the best-rated books.",
                true,
            ),
            "read",
        ),
        (
            t::<AuthorBooksArgs>(
                "list_author_books",
                "Books of an author",
                "All books of an author in bibliography order (series, number, title) or by a rating. Paginated.",
                true,
            ),
            "read",
        ),
        (
            t::<SeriesArg>(
                "get_series",
                "Series",
                "A series (by id or name) with its books in order, my status for each (rated, sent/read) and the next unread number.",
                true,
            ),
            "read",
        ),
        (
            t::<GenresArgs>(
                "list_genres",
                "Genres",
                "The genre tree with book counts in this library (ids for search_books / suggest_candidates).",
                true,
            ),
            "read",
        ),
        (
            t::<LibraryArg>(
                "get_reading_profile",
                "My reading profile",
                "My shelves with books, my ratings, my recent sends / downloads / reads, and my top genres and authors derived from them.",
                true,
            ),
            "read",
        ),
        (
            t::<SuggestArgs>(
                "suggest_candidates",
                "Suggest candidates",
                "Scored reading candidates with reasons (same author, next unread number of a series, shared genres, high library / Open Library rating). Seeds: `seed_book_ids` and/or my profile. Excludes books I rated, shelved, sent, downloaded or read unless told otherwise. You make the final choice.",
                true,
            ),
            "read",
        ),
        (
            t::<BookArg>(
                "get_external_rating",
                "Open Library rating",
                "The Open Library rating of a book (average and vote count, with a link); looks it up now when not cached (may take a few seconds).",
                true,
            ),
            "read",
        ),
        (
            t::<Empty>(
                "list_devices",
                "My devices",
                "My devices for send_books (e-mail like Send to Kindle, download, server folder) in my order; the first is the default.",
                true,
            ),
            "read",
        ),
        (
            t::<Empty>(
                "list_shelves",
                "My shelves",
                "My shelves with book counts.",
                true,
            ),
            "read",
        ),
        (
            t::<ShelfArgs>(
                "add_to_shelf",
                "Add to shelf",
                "Put books on one of my shelves (by id, or by name; `create` makes a missing shelf).",
                false,
            ),
            "write",
        ),
        (
            t::<ShelfArgs>(
                "remove_from_shelf",
                "Remove from shelf",
                "Take books off one of my shelves.",
                false,
            ),
            "write",
        ),
        (
            t::<RateArgs>(
                "rate_book",
                "Rate a book",
                "Set my rating of a book: 1..5 stars, 0 removes it.",
                false,
            ),
            "write",
        ),
        (
            t::<SendArgs>(
                "send_books",
                "Send books",
                "Send books to one of my devices (Send to Kindle by e-mail, a download, or a server folder). Respects the server's allowed recipients and daily mail limit. Returns a job id for get_job.",
                false,
            ),
            "send",
        ),
        (
            t::<JobArg>(
                "get_job",
                "Job status",
                "State of a send job (queued, running, done, failed) with its log.",
                true,
            ),
            "send",
        ),
    ]
}

/// The scope a tool needs.
pub fn scope_of(name: &str) -> Option<&'static str> {
    definitions()
        .into_iter()
        .find(|(t, _)| t.name == name)
        .map(|(_, s)| s)
}

fn parse<P: DeserializeOwned>(v: Value) -> Result<P, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid arguments: {e}"))
}

/// Runs a tool.
pub async fn call(st: &AppState, auth: &TokenAuth, name: &str, args: Value) -> ToolResult {
    match name {
        "list_libraries" => list_libraries(st, auth).await,
        "search_books" => search_books(st, auth, parse(args)?).await,
        "get_book" => get_book(st, auth, parse(args)?).await,
        "get_author" => get_author(st, auth, parse(args)?).await,
        "list_author_books" => list_author_books(st, auth, parse(args)?).await,
        "get_series" => get_series(st, auth, parse(args)?).await,
        "list_genres" => list_genres(st, auth, parse(args)?).await,
        "get_reading_profile" => reading_profile(st, auth, parse(args)?).await,
        "suggest_candidates" => suggest_candidates(st, auth, parse(args)?).await,
        "get_external_rating" => external_rating(st, auth, parse(args)?).await,
        "list_devices" => list_devices(st, auth).await,
        "list_shelves" => list_shelves(st, auth).await,
        "add_to_shelf" => shelf_change(st, auth, parse(args)?, true).await,
        "remove_from_shelf" => shelf_change(st, auth, parse(args)?, false).await,
        "rate_book" => rate_book(st, auth, parse(args)?).await,
        "send_books" => send_books(st, auth, parse(args)?).await,
        "get_job" => get_job(st, auth, parse(args)?).await,
        _ => Err(format!("unknown tool {name}")),
    }
}

// ---------------------------------------------------------------- arguments

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Empty {}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct LibraryArg {
    /// Library id (default: the default library).
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Words to find in titles, authors, series and keywords; each word matches as a prefix.
    pub query: Option<String>,
    /// Author name (words of it). With `query` it narrows the search; without, lists that author's books.
    pub author: Option<String>,
    /// Author id (from a previous result).
    pub author_id: Option<i64>,
    /// Series name.
    pub series: Option<String>,
    /// Series id.
    pub series_id: Option<i64>,
    /// Genre id (see list_genres); a top-level genre includes its sub-genres.
    pub genre_id: Option<u16>,
    /// Genre name, e.g. "Fantasy" or "Детская литература" (the best matching genre is used).
    pub genre: Option<String>,
    /// Language code, e.g. "ru", "en", "uk".
    pub language: Option<String>,
    /// Only books added on or after this date (YYYY-MM-DD).
    pub added_after: Option<String>,
    /// Only books added on or before this date (YYYY-MM-DD).
    pub added_before: Option<String>,
    /// Minimum of my own rating (1..5).
    pub min_my_rating: Option<u8>,
    /// Minimum library (INPX) rating (1..5).
    pub min_library_rating: Option<u8>,
    /// Minimum Open Library average (0..5).
    pub min_openlibrary_rating: Option<f64>,
    /// Minimum Open Library vote count.
    pub min_openlibrary_votes: Option<u32>,
    /// Only books I have not rated.
    pub unrated_by_me: Option<bool>,
    /// Only books whose (heuristic) age estimate is known and at most this age.
    pub kids_max_age: Option<u8>,
    /// relevance | date | my_rating | library_rating | openlibrary_rating
    pub sort: Option<String>,
    /// Books per page (default 20, max 50).
    pub limit: Option<usize>,
    /// `nextCursor` of the previous page.
    pub cursor: Option<String>,
    /// Library id (default: the default library).
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct BookArg {
    /// Book id.
    pub id: i64,
    /// Library id (default: the default library).
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AuthorArg {
    /// Author id.
    pub id: Option<i64>,
    /// Author name when the id is unknown (the author with most books among the matches).
    pub name: Option<String>,
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AuthorBooksArgs {
    /// Author id.
    pub author_id: i64,
    /// bibliography (default) | my_rating | library_rating | openlibrary_rating
    pub sort: Option<String>,
    /// Books per page (default 50, max 100).
    pub limit: Option<usize>,
    pub cursor: Option<String>,
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SeriesArg {
    /// Series id.
    pub id: Option<i64>,
    /// Series name when the id is unknown.
    pub name: Option<String>,
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GenresArgs {
    /// Only this top-level genre and its sub-genres.
    pub parent: Option<u16>,
    /// Name language: en (default), ru, uk.
    pub lang: Option<String>,
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SuggestArgs {
    /// Books to find similar ones to.
    #[serde(default)]
    pub seed_book_ids: Vec<i64>,
    /// Also use my profile (ratings ≥ 4, shelves, sends, downloads, reads). Default: true when there are no seed books.
    pub use_profile: Option<bool>,
    /// Only these genre ids (top-level ids include their sub-genres).
    #[serde(default)]
    pub genres: Vec<u16>,
    /// Only this language, e.g. "ru".
    pub language: Option<String>,
    /// Only books whose age estimate is known and at most this.
    pub kids_max_age: Option<u8>,
    /// Minimum library rating (1..5).
    pub min_library_rating: Option<u8>,
    /// Minimum Open Library average (0..5).
    pub min_openlibrary_rating: Option<f64>,
    /// Leave out books I rated, shelved, sent, downloaded or read (default true).
    pub exclude_read: Option<bool>,
    /// Candidates to return (default 20, max 50).
    pub limit: Option<usize>,
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ShelfArgs {
    /// Shelf id.
    pub shelf_id: Option<i64>,
    /// Shelf name (when the id is unknown).
    pub shelf: Option<String>,
    /// add_to_shelf only: create the shelf when no shelf has this name.
    pub create: Option<bool>,
    /// Book ids (at most 500).
    pub book_ids: Vec<i64>,
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RateArgs {
    /// Book id.
    pub id: i64,
    /// 1..5 stars, 0 removes my rating.
    pub rating: i64,
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SendArgs {
    /// Book ids (at most 50).
    pub book_ids: Vec<i64>,
    /// Device id from list_devices, or "default".
    pub device: Value,
    pub library: Option<i64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct JobArg {
    /// Job id from send_books.
    pub id: String,
}

// ---------------------------------------------------------------- helpers

/// The library to use: the given one (must exist), else the default, else the first imported.
fn resolve_lib(st: &AppState, lib: Option<i64>) -> Result<i64, String> {
    if let Some(id) = lib {
        st.catalog(id).map_err(err)?;
        return Ok(id);
    }
    let rows = {
        let c = st.db.lock();
        db::list_libraries(&c).map_err(err)?
    };
    rows.iter()
        .filter(|r| st.catalog(r.id).is_ok())
        .max_by_key(|r| (r.is_default, -r.id))
        .map(|r| r.id)
        .ok_or_else(|| "no library has been imported yet".to_string())
}

fn genre_name(id: u16) -> String {
    genres()
        .get(id)
        .map(|g| g.localized("en").to_string())
        .unwrap_or_else(|| id.to_string())
}

fn kids_label(a: Option<u8>) -> Value {
    a.map(|a| json!(format!("{a}+"))).unwrap_or(Value::Null)
}

/// Compact book JSON for lists.
fn book_json(b: &BookOut) -> Value {
    let bk = &b.book;
    json!({
        "id": bk.id,
        "title": bk.title,
        "authors": bk.authors.iter().map(|a| json!({"id": a.id, "name": a.name})).collect::<Vec<_>>(),
        "series": bk.series.as_ref().map(|s| json!({"id": s.id, "name": s.name, "number": bk.serno})),
        "genres": bk.genres.iter().map(|g| json!({"id": g, "name": genre_name(*g)})).collect::<Vec<_>>(),
        "language": bk.lang,
        "added": bk.date,
        "myRating": b.rating,
        "libraryRating": bk.lib_rating,
        "openLibrary": b.ext_rating.map(|e| json!({"avg": e.avg, "votes": e.votes})),
        "kidsAge": kids_label(bk.kids_age),
    })
}

fn offset_of(cursor: &Option<String>) -> Result<usize, String> {
    match cursor.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        None => Ok(0),
        Some(c) => c.parse().map_err(|_| "invalid cursor".to_string()),
    }
}

fn sort_of(s: Option<&str>) -> RatingSort {
    match s.unwrap_or("") {
        "my_rating" | "my" => RatingSort::My,
        "library_rating" | "lib" => RatingSort::Lib,
        "openlibrary_rating" | "ext" | "open_library_rating" => RatingSort::Ext,
        _ => RatingSort::None,
    }
}

fn check_date(d: &Option<String>, name: &str) -> Result<Option<String>, String> {
    match d.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(v) => {
            let p = freelib_catalog::util::parse_date(v);
            if p.is_empty() {
                Err(format!("{name} must be YYYY-MM-DD"))
            } else {
                Ok(Some(p))
            }
        }
    }
}

fn date_num(d: &str) -> u32 {
    d.chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// Author id by name: the author with most books among the full-text matches.
fn find_author(cat: &Catalog, name: &str) -> ApiResult<Option<(i64, String)>> {
    let Some(f) = freelib_catalog::search::fts_query(name) else {
        return Ok(None);
    };
    let conn = cat.conn()?;
    let mut st = conn.prepare_cached(
        "SELECT a.id, a.name FROM author a WHERE a.id IN (SELECT rowid FROM author_fts WHERE author_fts MATCH ?1) \
         ORDER BY a.book_count DESC LIMIT 1",
    )?;
    use rusqlite::OptionalExtension;
    Ok(st
        .query_row([f], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

fn find_series(cat: &Catalog, name: &str) -> ApiResult<Option<(i64, String)>> {
    let Some(f) = freelib_catalog::search::fts_query(name) else {
        return Ok(None);
    };
    let conn = cat.conn()?;
    let mut st = conn.prepare_cached(
        "SELECT s.id, s.name FROM series s WHERE s.id IN (SELECT rowid FROM series_fts WHERE series_fts MATCH ?1) \
         ORDER BY s.book_count DESC LIMIT 1",
    )?;
    use rusqlite::OptionalExtension;
    Ok(st
        .query_row([f], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

/// Genre id by name (any language; exact name first, then a name containing the text).
pub fn find_genre(name: &str) -> Option<u16> {
    let n = freelib_catalog::normalize(name);
    if n.is_empty() {
        return None;
    }
    let all = genres().all();
    let names = |g: &freelib_catalog::GenreDef| {
        [
            freelib_catalog::normalize(&g.name),
            freelib_catalog::normalize(&g.name_en),
            freelib_catalog::normalize(&g.name_uk),
        ]
    };
    all.iter()
        .find(|g| names(g).contains(&n))
        .or_else(|| {
            // prefer top-level groups, then the shortest name
            let mut c: Vec<&freelib_catalog::GenreDef> = all
                .iter()
                .filter(|g| names(g).iter().any(|x| !x.is_empty() && x.contains(&n)))
                .collect();
            c.sort_by_key(|g| (g.parent != 0, g.name_en.len()));
            c.into_iter().next()
        })
        .or_else(|| {
            all.iter()
                .find(|g| g.keys.iter().any(|k| *k == name.trim()))
        })
        .map(|g| g.id)
}

/// Plain text of the sanitized annotation HTML.
pub fn html_to_text(h: &str) -> String {
    let s = h
        .replace("</p>", "\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    let lines: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let text = lines.join("\n");
    if text.chars().count() > 4000 {
        let mut t: String = text.chars().take(4000).collect();
        t.push('…');
        t
    } else {
        text
    }
}

// ---------------------------------------------------------------- tools

async fn list_libraries(st: &AppState, auth: &TokenAuth) -> ToolResult {
    let rows = st.db.run(|c| db::list_libraries(c)).await.map_err(err)?;
    let default = resolve_lib(st, None).ok();
    let _ = auth;
    Ok(json!({
        "libraries": rows.iter().map(|r| {
            let d = st.library_base_dto(r);
            json!({"id": r.id, "name": r.name, "books": d.book_count, "authors": d.author_count,
                   "series": d.series_count, "imported": d.imported_at.is_some(), "default": Some(r.id) == default})
        }).collect::<Vec<_>>()
    }))
}

async fn search_books(st: &AppState, auth: &TokenAuth, a: SearchArgs) -> ToolResult {
    let lib = resolve_lib(st, a.library)?;
    let limit = a.limit.unwrap_or(20).clamp(1, 50);
    let offset = offset_of(&a.cursor)?;
    let from = check_date(&a.added_after, "added_after")?;
    let to = check_date(&a.added_before, "added_before")?;
    let rq = RatingQuery {
        sort: sort_of(a.sort.as_deref()),
        min_my: a.min_my_rating.unwrap_or(0).min(5),
        min_lib: a.min_library_rating.unwrap_or(0).min(5),
        min_ext: a
            .min_openlibrary_rating
            .map(|x| (x.clamp(0.0, 5.0) * 100.0).round() as u16)
            .unwrap_or(0),
        min_ext_votes: a.min_openlibrary_votes.unwrap_or(0),
        unrated_by_me: a.unrated_by_me.unwrap_or(false),
        kids_max_age: a.kids_max_age,
    };
    let by_date = a.sort.as_deref() == Some("date");
    let genre = match (a.genre_id, a.genre.as_deref()) {
        (Some(g), _) => Some(g),
        (None, Some(n)) => Some(find_genre(n).ok_or_else(|| format!("no genre matches \"{n}\""))?),
        _ => None,
    };
    let langs: Vec<String> = a
        .language
        .iter()
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty())
        .collect();
    let query = a
        .query
        .clone()
        .map(|q| q.trim().to_string())
        .filter(|q| q.chars().count() >= 2);
    let (st2, uid) = (st.clone(), auth.user.id);
    let (a_author, a_author_id, a_series, a_series_id) =
        (a.author.clone(), a.author_id, a.series.clone(), a.series_id);
    let out = st
        .catalog_call(lib, move |cat| -> ApiResult<Value> {
            let (books, total, note): (Vec<Book>, usize, Option<String>) = if let Some(q) = &query {
                let mut qq = q.clone();
                for extra in [&a_author, &a_series].into_iter().flatten() {
                    qq.push(' ');
                    qq.push_str(extra);
                }
                let sq = SearchQuery {
                    q: qq,
                    kind: SearchKind::Books,
                    genres: genre.into_iter().collect(),
                    langs: langs.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    limit: if by_date { 1000 } else { (offset + limit).min(1000) },
                    rating: rq.clone(),
                    ..Default::default()
                };
                let r = with_rating_source(&st2, uid, lib, cat, &rq, |src| {
                    Ok(cat.search_rated(&sq, src)?)
                })?;
                let mut books = r.books;
                if by_date {
                    books.sort_by(|x, y| y.date.cmp(&x.date).then(x.id.cmp(&y.id)));
                }
                let total = r.total as usize;
                let page: Vec<Book> = books.into_iter().skip(offset).take(limit).collect();
                (page, total, None)
            } else {
                let mut note = None;
                let sel = if let Some(id) = a_author_id {
                    BookSelector::Author(id)
                } else if let Some(n) = &a_author {
                    let (id, name) = find_author(cat, n)?
                        .ok_or_else(|| ApiError::not_found(format!("no author matches \"{n}\"")))?;
                    note = Some(format!("author: {name} (id {id})"));
                    BookSelector::Author(id)
                } else if let Some(id) = a_series_id {
                    BookSelector::Series(id)
                } else if let Some(n) = &a_series {
                    let (id, name) = find_series(cat, n)?
                        .ok_or_else(|| ApiError::not_found(format!("no series matches \"{n}\"")))?;
                    note = Some(format!("series: {name} (id {id})"));
                    BookSelector::Series(id)
                } else if let Some(g) = genre {
                    BookSelector::Genre(g)
                } else if let Some(d) = &from {
                    BookSelector::Since(d.clone())
                } else {
                    return Err(ApiError::bad_request(
                        "give a query (at least 2 characters) or an author, series, genre or added_after",
                    ));
                };
                let filter = BookFilter {
                    langs: langs.clone(),
                    ..Default::default()
                };
                let attrs = cat.attrs()?;
                let mut ids = cat.selection_ids(&sel, &filter, &attrs)?;
                if let (Some(g), false) = (genre, matches!(sel, BookSelector::Genre(_))) {
                    let set: HashSet<u16> = genres().with_descendants(g).into_iter().collect();
                    ids.retain(|id| attrs.genres(*id).iter().any(|x| set.contains(x)));
                }
                let (lo, hi) = (
                    from.as_deref().map(date_num).unwrap_or(0),
                    to.as_deref().map(date_num).unwrap_or(u32::MAX),
                );
                if lo > 0 || hi < u32::MAX {
                    ids.retain(|id| {
                        let d = attrs.date(*id);
                        d >= lo && d <= hi
                    });
                }
                if by_date {
                    ids.sort_by(|x, y| attrs.date(*y).cmp(&attrs.date(*x)).then(x.cmp(y)));
                }
                with_rating_source(&st2, uid, lib, cat, &rq, |src| {
                    rq.apply(&mut ids, &attrs, src);
                    Ok(())
                })?;
                let total = ids.len();
                let page: Vec<i64> = ids.into_iter().skip(offset).take(limit).collect();
                (cat.books_by_ids(&page)?, total, note)
            };
            let n = books.len();
            let marked = with_marks(&st2, uid, lib, books)?;
            Ok(json!({
                "library": lib,
                "total": total,
                "matched": note,
                "books": marked.iter().map(book_json).collect::<Vec<_>>(),
                "nextCursor": (offset + n < total && n > 0).then(|| (offset + n).to_string()),
            }))
        })
        .await
        .map_err(err)?;
    Ok(out)
}

async fn get_book(st: &AppState, auth: &TokenAuth, a: BookArg) -> ToolResult {
    let lib = resolve_lib(st, a.library)?;
    let (_, d) = crate::api::books::load_book(st, lib, a.id)
        .await
        .map_err(err)?;
    let dir = crate::api::books::lib_dir(st, lib).await.map_err(err)?;
    let info = crate::preview::info(st, lib, &dir, &d)
        .await
        .unwrap_or_default();
    let uid = auth.user.id;
    let (st2, book, key) = (st.clone(), d.book.clone(), d.book.key.clone());
    let marked = tokio::task::spawn_blocking(move || with_marks(&st2, uid, lib, vec![book]))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)?;
    let b = marked.into_iter().next().ok_or("book not found")?;
    let key2 = key.clone();
    let (shelves, history) = st
        .db
        .run(move |c| {
            let shelves = db::list_shelves(c, uid)?;
            let hist: Vec<db::HistoryRow> = db::history(c, uid, Some(lib), 5000)?
                .into_iter()
                .filter(|h| h.book_key == key2)
                .collect();
            Ok((shelves, hist))
        })
        .await
        .map_err(err)?;
    let ext = {
        let (e, k) = (st.ext.clone(), key.clone());
        tokio::task::spawn_blocking(move || e.entry(lib, &k))
            .await
            .map_err(|e| e.to_string())?
    };
    if ext.is_none() {
        st.ext
            .enqueue(crate::extrating::Priority::User, lib, &[a.id]);
    }
    let mut v = book_json(&b);
    let o = v.as_object_mut().ok_or("internal")?;
    o.insert("library".into(), json!(lib));
    o.insert("format".into(), json!(d.book.ext));
    o.insert("size".into(), json!(d.book.size));
    o.insert("deleted".into(), json!(d.book.deleted));
    o.insert("keywords".into(), json!(d.keywords));
    o.insert(
        "annotation".into(),
        json!(info.annotation.as_deref().map(html_to_text)),
    );
    o.insert(
        "formats".into(),
        json!(crate::output::formats_for(
            &d.book.ext,
            st.calibre.is_some()
        )),
    );
    o.insert(
        "openLibrary".into(),
        match &ext {
            Some(e) => json!({"status": e.status, "avg": e.average, "votes": e.count, "url": e.url, "fetchedAt": e.fetched_at}),
            None => json!({"status": "not_looked_up"}),
        },
    );
    o.insert(
        "shelves".into(),
        json!(
            b.shelves
                .iter()
                .filter_map(|id| shelves.iter().find(|s| s.id == *id))
                .map(|s| json!({"id": s.id, "name": s.name}))
                .collect::<Vec<_>>()
        ),
    );
    let did = |act: &str| {
        history
            .iter()
            .find(|h| h.action == act)
            .map(|h| h.at.clone())
    };
    o.insert(
        "myHistory".into(),
        json!({"lastSent": did("send"), "lastDownloaded": did("download"), "lastRead": did("read")}),
    );
    Ok(v)
}

async fn get_author(st: &AppState, auth: &TokenAuth, a: AuthorArg) -> ToolResult {
    let lib = resolve_lib(st, a.library)?;
    let (st2, uid) = (st.clone(), auth.user.id);
    st.catalog_call(lib, move |cat| -> ApiResult<Value> {
        let id = match (a.id, &a.name) {
            (Some(id), _) => id,
            (None, Some(n)) => {
                find_author(cat, n)?
                    .ok_or_else(|| ApiError::not_found(format!("no author matches \"{n}\"")))?
                    .0
            }
            _ => return Err(ApiError::bad_request("give id or name")),
        };
        let s = cat
            .author_summary(id)?
            .ok_or_else(|| ApiError::not_found("author not found"))?;
        // the best-rated books (library rating, then Open Library)
        let rq = RatingQuery {
            sort: RatingSort::Lib,
            ..Default::default()
        };
        let p = cat.books_rated(
            &BookSelector::Author(id),
            &BookFilter::default(),
            &rq,
            &freelib_catalog::NoRatings,
            &Page {
                cursor: None,
                limit: 10,
            },
        )?;
        let top = with_marks(&st2, uid, lib, p.books)?;
        Ok(json!({
            "library": lib,
            "id": s.id, "name": s.name, "books": s.count, "anthologies": s.anthologies,
            "series": s.series.iter().take(30).map(|x| json!({"id": x.id, "name": x.name, "books": x.count})).collect::<Vec<_>>(),
            "booksWithoutSeries": s.without_series,
            "languages": s.langs,
            "genres": s.genres.iter().map(|(g, n)| json!({"id": g, "name": genre_name(*g), "books": n})).collect::<Vec<_>>(),
            "years": [s.first_date, s.last_date],
            "coauthors": s.coauthors.iter().map(|c| json!({"id": c.id, "name": c.name, "sharedBooks": c.books})).collect::<Vec<_>>(),
            "bestRated": top.iter().map(book_json).collect::<Vec<_>>(),
        }))
    })
    .await
    .map_err(err)
}

async fn list_author_books(st: &AppState, auth: &TokenAuth, a: AuthorBooksArgs) -> ToolResult {
    let lib = resolve_lib(st, a.library)?;
    let limit = a.limit.unwrap_or(50).clamp(1, 100);
    let cursor = a.cursor.clone().filter(|c| !c.trim().is_empty());
    let (st2, uid) = (st.clone(), auth.user.id);
    let rq = RatingQuery {
        sort: sort_of(a.sort.as_deref()),
        ..Default::default()
    };
    st.catalog_call(lib, move |cat| -> ApiResult<Value> {
        let name = cat
            .author(a.author_id)?
            .ok_or_else(|| ApiError::not_found("author not found"))?
            .name;
        let page = Page {
            cursor: cursor.clone(),
            limit,
        };
        let p = with_rating_source(&st2, uid, lib, cat, &rq, |src| {
            Ok(cat.books_rated(
                &BookSelector::Author(a.author_id),
                &BookFilter::default(),
                &rq,
                src,
                &page,
            )?)
        })?;
        let books = with_marks(&st2, uid, lib, p.books)?;
        Ok(json!({
            "library": lib, "author": {"id": a.author_id, "name": name}, "total": p.total,
            "books": books.iter().map(book_json).collect::<Vec<_>>(), "nextCursor": p.next_cursor,
        }))
    })
    .await
    .map_err(err)
}

async fn get_series(st: &AppState, auth: &TokenAuth, a: SeriesArg) -> ToolResult {
    let lib = resolve_lib(st, a.library)?;
    let uid = auth.user.id;
    let hist = st
        .db
        .run(move |c| db::history_keys(c, uid, lib))
        .await
        .map_err(err)?;
    let st2 = st.clone();
    st.catalog_call(lib, move |cat| -> ApiResult<Value> {
        let id = match (a.id, &a.name) {
            (Some(id), _) => id,
            (None, Some(n)) => {
                find_series(cat, n)?
                    .ok_or_else(|| ApiError::not_found(format!("no series matches \"{n}\"")))?
                    .0
            }
            _ => return Err(ApiError::bad_request("give id or name")),
        };
        let s = cat
            .series(id)?
            .ok_or_else(|| ApiError::not_found("series not found"))?;
        let p = cat.books(
            &BookSelector::Series(id),
            &BookFilter::default(),
            &Page {
                cursor: None,
                limit: 500,
            },
        )?;
        let books = with_marks(&st2, uid, lib, p.books)?;
        let done = |b: &BookOut| {
            b.rating > 0 || hist.get(&b.book.key).is_some_and(|a| !a.is_empty())
        };
        let last_done = books
            .iter()
            .filter(|b| done(b))
            .filter_map(|b| b.book.serno)
            .max();
        let next = books
            .iter()
            .filter(|b| !done(b) && b.book.serno.is_some_and(|n| last_done.is_none_or(|l| n > l)))
            .min_by_key(|b| b.book.serno);
        Ok(json!({
            "library": lib, "id": s.id, "name": s.name, "authors": s.authors, "books": p.total,
            "items": books.iter().take(200).map(|b| {
                let mut v = book_json(b);
                if let Some(o) = v.as_object_mut() {
                    o.remove("series");
                    o.insert("number".into(), json!(b.book.serno));
                    o.insert("myActions".into(), json!(hist.get(&b.book.key).cloned().unwrap_or_default()));
                }
                v
            }).collect::<Vec<_>>(),
            "nextUnread": next.map(|b| json!({"id": b.book.id, "number": b.book.serno, "title": b.book.title})),
        }))
    })
    .await
    .map_err(err)
}

async fn list_genres(st: &AppState, _auth: &TokenAuth, a: GenresArgs) -> ToolResult {
    let lib = resolve_lib(st, a.library)?;
    let lang = match a.lang.as_deref() {
        Some("ru") => "ru",
        Some("uk") => "uk",
        _ => "en",
    };
    let parent = a.parent;
    st.catalog_call(lib, move |cat| -> ApiResult<Value> {
        let all = cat.genres(lang)?;
        let tops: Vec<Value> = all
            .iter()
            .filter(|g| g.parent == 0 && g.count > 0 && parent.is_none_or(|p| p == g.id))
            .map(|t| {
                let subs: Vec<Value> = all
                    .iter()
                    .filter(|g| g.parent == t.id && g.count > 0)
                    .map(|g| json!({"id": g.id, "name": g.name, "books": g.count}))
                    .collect();
                json!({"id": t.id, "name": t.name, "books": t.count, "subgenres": subs})
            })
            .collect();
        Ok(json!({"library": lib, "genres": tops}))
    })
    .await
    .map_err(err)
}

/// The user's marks in one library: ratings, shelves (id, name, key), history.
struct Marks {
    ratings: Vec<(String, i64)>,
    shelf_books: Vec<(i64, String, String, String)>,
    shelves: Vec<db::Shelf>,
    history: Vec<db::HistoryRow>,
}

async fn load_marks(st: &AppState, uid: i64, lib: i64) -> Result<Marks, String> {
    st.db
        .run(move |c| {
            Ok(Marks {
                ratings: db::user_ratings(c, uid, lib)?,
                shelf_books: db::user_shelf_books(c, uid, lib)?,
                shelves: db::list_shelves(c, uid)?,
                history: db::history(c, uid, Some(lib), 500)?,
            })
        })
        .await
        .map_err(err)
}

/// Seeds of the profile: liked books (and disliked, negative), with weights.
fn profile_seeds(m: &Marks) -> Vec<(String, f64, String)> {
    let mut w: HashMap<String, (f64, String)> = HashMap::new();
    let mut add = |k: &str, x: f64, why: String| {
        let e = w.entry(k.to_string()).or_insert((0.0, why.clone()));
        e.0 += x;
    };
    for (k, r) in &m.ratings {
        let x = match r {
            5 => 3.0,
            4 => 2.0,
            3 => 0.5,
            _ => -2.0,
        };
        add(k, x, format!("rated {r}"));
    }
    for (_, name, k, _) in &m.shelf_books {
        add(k, 1.0, format!("on shelf {name}"));
    }
    for h in &m.history {
        add(
            &h.book_key,
            1.0,
            format!("{} {}", h.action, &h.at[..10.min(h.at.len())]),
        );
    }
    // several sends of one book count once more at most
    w.into_iter()
        .map(|(k, (x, why))| (k, x.clamp(-3.0, 5.0), why))
        .collect()
}

async fn reading_profile(st: &AppState, auth: &TokenAuth, a: LibraryArg) -> ToolResult {
    let lib = resolve_lib(st, a.library)?;
    let uid = auth.user.id;
    let m = Arc::new(load_marks(st, uid, lib).await?);
    let st2 = st.clone();
    st.catalog_call(lib, move |cat| -> ApiResult<Value> {
        let mut keys: Vec<String> = m.ratings.iter().map(|r| r.0.clone()).collect();
        keys.extend(m.shelf_books.iter().map(|s| s.2.clone()));
        keys.extend(m.history.iter().map(|h| h.book_key.clone()));
        keys.sort();
        keys.dedup();
        let ids: HashMap<String, i64> = cat.ids_by_keys(&keys)?.into_iter().collect();
        let all_ids: Vec<i64> = ids.values().copied().collect();
        let books: HashMap<i64, BookOut> = with_marks(&st2, uid, lib, cat.books_by_ids(&all_ids)?)?
            .into_iter()
            .map(|b| (b.book.id, b))
            .collect();
        let by_key = |k: &str| ids.get(k).and_then(|id| books.get(id));
        let brief = |b: &BookOut| {
            json!({"id": b.book.id, "title": b.book.title,
                   "authors": b.book.authors.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
                   "series": b.book.series.as_ref().map(|s| format!("{} #{}", s.name, b.book.serno.map(|n| n.to_string()).unwrap_or_default())),
                   "myRating": b.rating})
        };
        let shelves: Vec<Value> = m
            .shelves
            .iter()
            .take(10)
            .map(|s| {
                let items: Vec<Value> = m
                    .shelf_books
                    .iter()
                    .filter(|x| x.0 == s.id)
                    .filter_map(|x| by_key(&x.2))
                    .take(20)
                    .map(brief)
                    .collect();
                json!({"id": s.id, "name": s.name, "books": s.count, "items": items})
            })
            .collect();
        let mut rated: Vec<&(String, i64)> = m.ratings.iter().collect();
        rated.sort_by_key(|r| std::cmp::Reverse(r.1));
        let ratings: Vec<Value> = rated
            .iter()
            .filter_map(|(k, r)| by_key(k).map(|b| (b, *r)))
            .take(50)
            .map(|(b, _)| brief(b))
            .collect();
        let history: Vec<Value> = m
            .history
            .iter()
            .filter_map(|h| by_key(&h.book_key).map(|b| (h, b)))
            .take(30)
            .map(|(h, b)| json!({"action": h.action, "at": h.at, "device": h.device, "book": brief(b)}))
            .collect();
        // derived tastes
        let mut gw: HashMap<u16, f64> = HashMap::new();
        let mut aw: HashMap<i64, (f64, String)> = HashMap::new();
        for (k, w, _) in profile_seeds(&m) {
            let Some(b) = by_key(&k) else { continue };
            if w > 0.0 {
                for g in &b.book.genres {
                    *gw.entry(*g).or_default() += w / b.book.genres.len().max(1) as f64;
                }
            }
            if b.book.authors.len() < suggest::ANTHOLOGY_MIN_AUTHORS {
                for a in &b.book.authors {
                    aw.entry(a.id).or_insert((0.0, a.name.clone())).0 += w;
                }
            }
        }
        let mut tg: Vec<(u16, f64)> = gw.into_iter().collect();
        tg.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut ta: Vec<(i64, (f64, String))> = aw.into_iter().filter(|x| x.1.0 > 0.0).collect();
        ta.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
        Ok(json!({
            "library": lib,
            "user": auth_name(uid, &st2),
            "counts": {"rated": m.ratings.len(), "shelved": m.shelf_books.len(), "history": m.history.len()},
            "shelves": shelves,
            "ratings": ratings,
            "recent": history,
            "topGenres": tg.iter().take(8).map(|(g, w)| json!({"id": g, "name": genre_name(*g), "weight": (w * 10.0).round() / 10.0})).collect::<Vec<_>>(),
            "topAuthors": ta.iter().take(10).map(|(id, (w, n))| json!({"id": id, "name": n, "weight": (w * 10.0).round() / 10.0})).collect::<Vec<_>>(),
        }))
    })
    .await
    .map_err(err)
}

fn auth_name(uid: i64, st: &AppState) -> String {
    if uid == 0 {
        return "admin".into();
    }
    let c = st.db.lock();
    db::get_user(&c, uid)
        .ok()
        .flatten()
        .map(|u| u.username)
        .unwrap_or_default()
}

fn facts(b: &BookOut) -> BookFacts {
    BookFacts {
        id: b.book.id,
        title: b.book.title.clone(),
        authors: b
            .book
            .authors
            .iter()
            .map(|a| (a.id, a.name.clone()))
            .collect(),
        series: b
            .book
            .series
            .as_ref()
            .map(|s| (s.id, s.name.clone(), b.book.serno)),
        genres: b.book.genres.clone(),
        stars: b.book.lib_rating.clamp(0, 5) as u8,
        ext: b.ext_rating.map(|e| (e.avg, e.votes)),
    }
}

async fn suggest_candidates(st: &AppState, auth: &TokenAuth, a: SuggestArgs) -> ToolResult {
    let lib = resolve_lib(st, a.library)?;
    let uid = auth.user.id;
    let limit = a.limit.unwrap_or(20).clamp(1, 50);
    if a.seed_book_ids.len() > 100 {
        return Err("at most 100 seed books".into());
    }
    let use_profile = a.use_profile.unwrap_or(a.seed_book_ids.is_empty());
    let exclude_read = a.exclude_read.unwrap_or(true);
    let m = Arc::new(load_marks(st, uid, lib).await?);
    let st2 = st.clone();
    st.catalog_call(lib, move |cat| -> ApiResult<Value> {
        let attrs = cat.attrs()?;
        // seeds
        let mut seed_w: HashMap<i64, (f64, String)> = HashMap::new();
        for id in &a.seed_book_ids {
            seed_w.insert(*id, (3.0, "you asked".into()));
        }
        let mut known: HashSet<i64> = HashSet::new();
        {
            let mut keys: Vec<String> = m.ratings.iter().map(|r| r.0.clone()).collect();
            keys.extend(m.shelf_books.iter().map(|s| s.2.clone()));
            keys.extend(m.history.iter().map(|h| h.book_key.clone()));
            keys.sort();
            keys.dedup();
            let ids: HashMap<String, i64> = cat.ids_by_keys(&keys)?.into_iter().collect();
            known.extend(ids.values().copied());
            if use_profile {
                for (k, w, why) in profile_seeds(&m) {
                    if let Some(id) = ids.get(&k) {
                        let e = seed_w.entry(*id).or_insert((0.0, why));
                        e.0 += w;
                    }
                }
            }
        }
        if seed_w.is_empty() {
            return Err(ApiError::bad_request(
                "nothing to start from: give seed_book_ids, or rate / shelve / send some books first",
            ));
        }
        let seed_ids: Vec<i64> = seed_w.keys().copied().collect();
        let seed_books = with_marks(&st2, uid, lib, cat.books_by_ids(&seed_ids)?)?;
        let seeds: Vec<Seed> = seed_books
            .iter()
            .map(|b| {
                let (w, why) = seed_w.get(&b.book.id).cloned().unwrap_or((1.0, String::new()));
                Seed { book: facts(b), weight: w, why }
            })
            .collect();
        let profile = Profile::from_seeds(&seeds);
        // candidates: books of the top authors, the seed series, well-rated books of top genres
        let mut cand: Vec<i64> = Vec::new();
        {
            let conn = cat.conn()?;
            let top_authors = profile.top_authors(15);
            let mut st_a = conn.prepare_cached(
                "SELECT book_id FROM book_author WHERE author_id=?1 LIMIT 400",
            )?;
            for aid in &top_authors {
                let ids: Vec<i64> = st_a
                    .query_map([aid], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                cand.extend(ids);
            }
            let mut st_s =
                conn.prepare_cached("SELECT id FROM book WHERE series_id=?1 LIMIT 300")?;
            for sid in profile.series.keys() {
                let ids: Vec<i64> = st_s
                    .query_map([sid], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                cand.extend(ids);
            }
        }
        let dense = st2.ext.dense(lib, cat);
        let dguard = dense.as_ref().map(|d| d.read().unwrap_or_else(|e| e.into_inner()));
        let top_genres: HashSet<u16> = profile.top_genres(5).into_iter().collect();
        if !top_genres.is_empty() {
            // well-rated books of the favourite genres, best first, bounded
            let mut rated: Vec<(u32, i64)> = Vec::new();
            for id in 1..attrs.len() as i64 {
                if !attrs.is_live(id) || !attrs.genres(id).iter().any(|g| top_genres.contains(g)) {
                    continue;
                }
                let stars = attrs.stars(id) as u32;
                let ext = dguard.as_ref().and_then(|d| d.get(id));
                let good = stars >= 4 || ext.is_some_and(|(avg, v)| avg >= 400 && v >= 5);
                if good {
                    let key = stars * 100 + ext.map(|(a, _)| a as u32 / 10).unwrap_or(0);
                    rated.push((key, id));
                }
            }
            rated.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            cand.extend(rated.into_iter().take(1500).map(|x| x.1));
        }
        drop(dguard);
        // constraints and exclusions
        let gfilter: HashSet<u16> = a
            .genres
            .iter()
            .flat_map(|g| genres().with_descendants(*g))
            .collect();
        let lang = a.language.as_deref().map(|l| l.trim().to_lowercase());
        let rq = RatingQuery {
            min_lib: a.min_library_rating.unwrap_or(0).min(5),
            min_ext: a
                .min_openlibrary_rating
                .map(|x| (x.clamp(0.0, 5.0) * 100.0).round() as u16)
                .unwrap_or(0),
            kids_max_age: a.kids_max_age,
            ..Default::default()
        };
        let seed_set: HashSet<i64> = seed_ids.iter().copied().collect();
        let mut seen = HashSet::new();
        cand.retain(|id| {
            seen.insert(*id)
                && attrs.is_live(*id)
                && !seed_set.contains(id)
                && !(exclude_read && known.contains(id))
                && (gfilter.is_empty() || attrs.genres(*id).iter().any(|g| gfilter.contains(g)))
                && lang.as_deref().is_none_or(|l| attrs.lang(*id) == l)
        });
        with_rating_source(&st2, uid, lib, cat, &rq, |src| {
            rq.apply(&mut cand, &attrs, src);
            Ok(())
        })?;
        cand.truncate(3000);
        let cbooks = with_marks(&st2, uid, lib, cat.books_by_ids(&cand)?)?;
        let cfacts: Vec<BookFacts> = cbooks.iter().map(facts).collect();
        let scored = suggest::score(&profile, &cfacts, &genre_name, limit);
        let by_id: HashMap<i64, &BookOut> = cbooks.iter().map(|b| (b.book.id, b)).collect();
        Ok(json!({
            "library": lib,
            "seeds": seeds.iter().take(20).map(|s| json!({"id": s.book.id, "title": s.book.title, "weight": s.weight, "why": s.why})).collect::<Vec<_>>(),
            "candidatesConsidered": cfacts.len(),
            "candidates": scored.iter().filter_map(|s| by_id.get(&s.id).map(|b| {
                let mut v = book_json(b);
                if let Some(o) = v.as_object_mut() {
                    o.insert("score".into(), json!(s.score));
                    o.insert("reasons".into(), json!(s.reasons));
                }
                v
            })).collect::<Vec<_>>(),
        }))
    })
    .await
    .map_err(err)
}

async fn external_rating(st: &AppState, _auth: &TokenAuth, a: BookArg) -> ToolResult {
    let lib = resolve_lib(st, a.library)?;
    let e = st.ext.lookup_now(st, lib, a.id).await.map_err(err)?;
    Ok(json!({
        "library": lib, "id": a.id, "source": e.source, "status": e.status,
        "avg": e.average, "votes": e.count, "workKey": e.work_key, "url": e.url, "fetchedAt": e.fetched_at,
    }))
}

/// The default device: the first one in the user's order (Settings → Devices), like the web
/// app's quick-send button.
fn default_device(devices: &[db::Device]) -> Option<i64> {
    devices.first().map(|d| d.id)
}

async fn list_devices(st: &AppState, auth: &TokenAuth) -> ToolResult {
    let uid = auth.user.id;
    let devs = st
        .db
        .run(move |c| db::list_devices(c, uid))
        .await
        .map_err(err)?;
    let def = default_device(&devs);
    Ok(json!({
        "devices": devs.iter().map(|d| json!({
            "id": d.id, "name": d.name, "kind": d.kind, "format": d.format,
            "target": d.target, "shared": d.shared, "default": Some(d.id) == def,
        })).collect::<Vec<_>>(),
    }))
}

async fn list_shelves(st: &AppState, auth: &TokenAuth) -> ToolResult {
    let uid = auth.user.id;
    let s = st
        .db
        .run(move |c| db::list_shelves(c, uid))
        .await
        .map_err(err)?;
    Ok(json!({"shelves": s}))
}

async fn shelf_change(st: &AppState, auth: &TokenAuth, a: ShelfArgs, add: bool) -> ToolResult {
    if a.book_ids.is_empty() || a.book_ids.len() > 500 {
        return Err("give 1..500 book_ids".into());
    }
    let lib = resolve_lib(st, a.library)?;
    let uid = auth.user.id;
    let (sid, name, create) = (
        a.shelf_id,
        a.shelf.clone(),
        a.create.unwrap_or(false) && add,
    );
    let shelf = st
        .db
        .run(move |c| {
            if let Some(id) = sid {
                return db::get_shelf(c, uid, id);
            }
            let n = name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or_else(|| ApiError::bad_request("give shelf_id or shelf"))?;
            if let Some(s) = db::list_shelves(c, uid)?
                .into_iter()
                .find(|s| s.name.eq_ignore_ascii_case(n))
            {
                return Ok(s);
            }
            if create && n.chars().count() <= 100 {
                return db::create_shelf(c, uid, n, "#1F5F5B");
            }
            Err(ApiError::not_found(format!(
                "no shelf named \"{n}\" (pass create=true to make it)"
            )))
        })
        .await
        .map_err(err)?;
    let ids = a.book_ids.clone();
    let keys: Vec<String> = st
        .catalog_call(lib, move |cat| Ok(cat.keys_by_ids(&ids)?))
        .await
        .map_err(err)?
        .into_iter()
        .map(|(_, k)| k)
        .collect();
    if keys.is_empty() {
        return Err("none of these books exist".into());
    }
    if add {
        st.ext
            .enqueue(crate::extrating::Priority::User, lib, &a.book_ids);
    }
    let (n, shelf_id) = (keys.len(), shelf.id);
    let shelf = st
        .db
        .run(move |c| {
            db::shelf_modify(c, shelf_id, lib, &keys, add)?;
            db::get_shelf(c, uid, shelf_id)
        })
        .await
        .map_err(err)?;
    Ok(json!({"shelf": shelf, "changed": n, "library": lib}))
}

async fn rate_book(st: &AppState, auth: &TokenAuth, a: RateArgs) -> ToolResult {
    if !(0..=5).contains(&a.rating) {
        return Err("rating must be 0..5".into());
    }
    let lib = resolve_lib(st, a.library)?;
    let id = a.id;
    let key = st
        .catalog_call(lib, move |cat| Ok(cat.keys_by_ids(&[id])?))
        .await
        .map_err(err)?
        .pop()
        .map(|(_, k)| k)
        .ok_or("book not found")?;
    let (uid, r) = (auth.user.id, a.rating);
    st.db
        .run(move |c| db::set_rating(c, uid, lib, &key, r))
        .await
        .map_err(err)?;
    st.ext.enqueue(crate::extrating::Priority::User, lib, &[id]);
    Ok(json!({"id": id, "library": lib, "myRating": r}))
}

async fn send_books(st: &AppState, auth: &TokenAuth, a: SendArgs) -> ToolResult {
    if a.book_ids.is_empty() || a.book_ids.len() > 50 {
        return Err("give 1..50 book_ids".into());
    }
    let lib = resolve_lib(st, a.library)?;
    let uid = auth.user.id;
    let devs = st
        .db
        .run(move |c| db::list_devices(c, uid))
        .await
        .map_err(err)?;
    let dev_id = match &a.device {
        Value::Number(n) => n.as_i64().ok_or("device must be an id or \"default\"")?,
        Value::String(s) if s == "default" || s.is_empty() => {
            default_device(&devs).ok_or("no devices configured")?
        }
        Value::String(s) => s
            .parse()
            .map_err(|_| "device must be an id or \"default\"")?,
        Value::Null => default_device(&devs).ok_or("no devices configured")?,
        _ => return Err("device must be an id or \"default\"".into()),
    };
    let device = devs
        .into_iter()
        .find(|d| d.id == dev_id)
        .ok_or("device not found")?;
    let req = crate::sender::SendRequest {
        library: lib,
        books: a.book_ids.clone(),
        device,
        target: None,
        file_name: None,
    };
    let job = crate::sender::start(st, &auth.user, req)
        .await
        .map_err(err)?;
    Ok(json!({"jobId": job.id, "state": job.state, "title": job.title}))
}

async fn get_job(st: &AppState, auth: &TokenAuth, a: JobArg) -> ToolResult {
    let j = st.jobs.get(&a.id, &auth.user).ok_or("job not found")?;
    Ok(json!({
        "id": j.id, "kind": j.kind, "title": j.title, "state": j.state, "progress": j.progress,
        "message": j.message, "log": j.log, "createdAt": j.created_at, "finishedAt": j.finished_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html() {
        assert_eq!(
            html_to_text("<p>One &amp; <em>two</em></p><p>Three<br>four</p>"),
            "One & two\nThree\nfour"
        );
    }

    #[test]
    fn genre_lookup() {
        assert!(find_genre("Fantasy").is_some());
        assert_eq!(find_genre("Детское"), Some(7));
        assert!(find_genre("").is_none());
    }

    #[test]
    fn every_tool_has_a_scope() {
        let d = definitions();
        assert_eq!(d.len(), 17);
        for (t, s) in &d {
            assert!(["read", "write", "send"].contains(s), "{}", t.name);
            assert!(t.input_schema.get("type").is_some(), "{}", t.name);
        }
        assert_eq!(scope_of("send_books"), Some("send"));
        assert_eq!(scope_of("rate_book"), Some("write"));
        assert_eq!(scope_of("nope"), None);
    }
}
