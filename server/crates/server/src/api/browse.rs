use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use freelib_catalog::{
    Book, BookFilter, BookSelector, Catalog, Page, RatingQuery, RatingSort, RatingSource,
    SearchKind, SearchQuery,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::auth::Auth;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, CachedBody, LibRuntime};
use crate::util::{etag_matches, json_etag, not_modified, preferred_encoding, set_header};

use super::split_list;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "private, no-cache";

/// External rating of a book in lists: Open Library average and vote count.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtRatingOut {
    pub avg: f64,
    pub votes: u32,
}

/// API `Book` (catalog book + the user's rating and shelves + the external rating).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookOut {
    #[serde(flatten)]
    pub book: Book,
    pub rating: i64,
    pub shelves: Vec<i64>,
    /// Open Library rating (only when the book was found and has votes).
    pub ext_rating: Option<ExtRatingOut>,
}

/// Adds rating and shelves of `user_id` and the cached external rating (blocking).
pub fn with_marks(
    st: &AppState,
    user_id: i64,
    lib: i64,
    books: Vec<Book>,
) -> ApiResult<Vec<BookOut>> {
    let keys: Vec<String> = books.iter().map(|b| b.key.clone()).collect();
    let (ratings, mut shelves) = {
        let c = st.db.lock();
        db::user_marks(&c, user_id, lib, &keys)?
    };
    Ok(books
        .into_iter()
        .map(|b| BookOut {
            rating: ratings.get(&b.key).copied().unwrap_or(0),
            shelves: shelves.remove(&b.key).unwrap_or_default(),
            ext_rating: st
                .ext
                .get(lib, &b.key)
                .map(|(avg, votes)| ExtRatingOut { avg, votes }),
            book: b,
        })
        .collect())
}

/// Rating filter / sort query parameters (books, search).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RatingParams {
    /// `my`, `lib`, `ext` (rating sorts); anything else keeps the list order.
    pub sort: Option<String>,
    pub min_my: Option<String>,
    pub min_lib: Option<String>,
    pub min_ext: Option<String>,
    pub min_ext_votes: Option<String>,
    pub unrated_by_me: Option<String>,
    pub kids_max_age: Option<String>,
}

/// Parses an optional number parameter (empty = absent).
fn num<T: std::str::FromStr>(v: &Option<String>, name: &str) -> ApiResult<Option<T>> {
    match v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => s
            .parse()
            .map(Some)
            .map_err(|_| ApiError::bad_request(format!("{name} must be a number"))),
    }
}

impl RatingParams {
    pub fn query(&self) -> ApiResult<RatingQuery> {
        let stars = |v: &Option<String>, name: &str| -> ApiResult<u8> {
            match num::<u8>(v, name)? {
                None => Ok(0),
                Some(n) if n <= 5 => Ok(n),
                Some(_) => Err(ApiError::bad_request(format!("{name} must be 0..5"))),
            }
        };
        let min_ext = match num::<f64>(&self.min_ext, "minExt")? {
            None => 0,
            Some(x) if (0.0..=5.0).contains(&x) => (x * 100.0).round() as u16,
            Some(_) => return Err(ApiError::bad_request("minExt must be 0..5")),
        };
        Ok(RatingQuery {
            sort: RatingSort::parse(self.sort.as_deref().unwrap_or("")),
            min_my: stars(&self.min_my, "minMy")?,
            min_lib: stars(&self.min_lib, "minLib")?,
            min_ext,
            min_ext_votes: num::<u32>(&self.min_ext_votes, "minExtVotes")?.unwrap_or(0),
            unrated_by_me: truthy(&self.unrated_by_me),
            kids_max_age: num::<u8>(&self.kids_max_age, "kidsMaxAge")?.map(|a| a.min(18)),
        })
    }
}

/// Runs `f` with the rating sources `rq` needs: the user's ratings of `lib` (by book id) and
/// the external ratings of this catalog version (blocking).
pub fn with_rating_source<R>(
    st: &AppState,
    user_id: i64,
    lib: i64,
    cat: &Catalog,
    rq: &RatingQuery,
    f: impl FnOnce(&dyn RatingSource) -> ApiResult<R>,
) -> ApiResult<R> {
    with_sources(st, user_id, lib, cat, rq, false, f)
}

/// [`with_rating_source`], plus the known covers when editions are grouped (`group`).
pub fn with_sources<R>(
    st: &AppState,
    user_id: i64,
    lib: i64,
    cat: &Catalog,
    rq: &RatingQuery,
    group: bool,
    f: impl FnOnce(&dyn RatingSource) -> ApiResult<R>,
) -> ApiResult<R> {
    let needs_my = rq.min_my > 0 || rq.unrated_by_me || rq.sort == RatingSort::My;
    let needs_ext = rq.min_ext > 0 || rq.min_ext_votes > 0 || rq.sort == RatingSort::Ext;
    let mut my = std::collections::HashMap::new();
    if needs_my {
        let rows = {
            let c = st.db.lock();
            db::user_ratings(&c, user_id, lib)?
        };
        let by_key: std::collections::HashMap<String, i64> = rows.into_iter().collect();
        let keys: Vec<String> = by_key.keys().cloned().collect();
        for (k, id) in cat.ids_by_keys(&keys)? {
            if let Some(r) = by_key.get(&k) {
                my.insert(id, (*r).clamp(0, 5) as u8);
            }
        }
    }
    let dense = if needs_ext {
        st.ext.dense(lib, cat)
    } else {
        None
    };
    let guard = dense
        .as_ref()
        .map(|d| d.read().unwrap_or_else(|e| e.into_inner()));
    // covers only matter for the best copy of grouped editions
    let covers = if group {
        st.covers.ids(lib, cat)
    } else {
        std::collections::HashMap::new()
    };
    let src = crate::extrating::Ratings {
        my,
        ext: guard,
        covers,
    };
    f(&src)
}

#[derive(Deserialize)]
pub struct VersionQuery {
    v: Option<i64>,
}

/// Builds (or returns the cached) authors / series list body. Blocking.
pub fn build_list(
    rt: &LibRuntime,
    cat: &Catalog,
    kind: &'static str,
) -> ApiResult<Arc<CachedBody>> {
    let version = cat.catalog_version();
    if let Some(b) = rt.cached_list(kind, version) {
        return Ok(b);
    }
    let _g = rt.build_lock.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(b) = rt.cached_list(kind, version) {
        return Ok(b);
    }
    let list = rt.name_list(cat, kind)?;
    #[derive(Serialize)]
    struct Out<'a> {
        version: i64,
        columns: [&'static str; 3],
        rows: &'a [(i64, String, i64)],
        letters: &'a [(String, i64, i64)],
    }
    let json = serde_json::to_vec(&Out {
        version,
        columns: ["id", "name", "count"],
        rows: &list.rows,
        letters: &list.letters,
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let body = Arc::new(CachedBody::new(
        version,
        format!("W/\"{kind}-{}-{version}\"", rt.id),
        json,
    ));
    rt.put_list(kind, body.clone());
    Ok(body)
}

async fn name_list(
    st: AppState,
    lib: i64,
    kind: &'static str,
    v: Option<i64>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let rt = st.lib(lib)?;
    let Some(cat) = rt.handle.get() else {
        return Ok(Json(
            json!({"version": 0, "columns": ["id", "name", "count"], "rows": [], "letters": []}),
        )
        .into_response());
    };
    let version = cat.catalog_version();
    let cache = if v == Some(version) {
        IMMUTABLE
    } else {
        REVALIDATE
    };
    let etag = format!("W/\"{kind}-{lib}-{version}\"");
    if etag_matches(&headers, &etag) {
        return Ok(not_modified(&etag, cache));
    }
    let enc = preferred_encoding(&headers);
    let bytes = tokio::task::spawn_blocking(move || -> ApiResult<bytes::Bytes> {
        let b = build_list(&rt, &cat, kind)?;
        Ok(b.encoded(enc))
    })
    .await??;
    let mut r = Response::new(Body::from(bytes));
    set_header(&mut r, header::CONTENT_TYPE, "application/json");
    set_header(&mut r, header::ETAG, &etag);
    set_header(&mut r, header::CACHE_CONTROL, cache);
    set_header(&mut r, header::VARY, "Accept-Encoding");
    if !enc.is_empty() {
        set_header(&mut r, header::CONTENT_ENCODING, enc);
    }
    Ok(r)
}

pub async fn authors(
    State(st): State<AppState>,
    _: Auth,
    Path(lib): Path<i64>,
    Query(q): Query<VersionQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    name_list(st, lib, "authors", q.v, headers).await
}

pub async fn series(
    State(st): State<AppState>,
    _: Auth,
    Path(lib): Path<i64>,
    Query(q): Query<VersionQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    name_list(st, lib, "series", q.v, headers).await
}

#[derive(Deserialize)]
pub struct GenresQuery {
    v: Option<i64>,
    lang: Option<String>,
}

/// Normalizes to "en", "ru" or "uk"; anything else (or missing) is "en".
fn genre_lang(lang: Option<&str>) -> &'static str {
    match lang {
        Some("ru") => "ru",
        Some("uk") => "uk",
        _ => "en",
    }
}

pub async fn genres(
    State(st): State<AppState>,
    _: Auth,
    Path(lib): Path<i64>,
    Query(q): Query<GenresQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let rt = st.lib(lib)?;
    let cat = rt.handle.get();
    let version = cat.as_ref().map(|c| c.catalog_version()).unwrap_or(0);
    let lang = genre_lang(q.lang.as_deref());
    let cache = if q.v == Some(version) && version > 0 {
        IMMUTABLE
    } else {
        REVALIDATE
    };
    let etag = format!("W/\"genres-{lib}-{version}-{lang}\"");
    if etag_matches(&headers, &etag) {
        return Ok(not_modified(&etag, cache));
    }
    let list = tokio::task::spawn_blocking(move || -> ApiResult<serde_json::Value> {
        match cat {
            Some(c) => Ok(serde_json::to_value(c.genres(lang)?).unwrap_or_default()),
            None => Ok(serde_json::to_value(
                freelib_catalog::genres()
                    .all()
                    .iter()
                    .map(|g| {
                        json!({"id": g.id, "name": g.localized(lang), "parent": g.parent, "count": 0})
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default()),
        }
    })
    .await??;
    let mut r = Json(list).into_response();
    set_header(&mut r, header::ETAG, &etag);
    set_header(&mut r, header::CACHE_CONTROL, cache);
    Ok(r)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BooksQuery {
    author: Option<i64>,
    series: Option<i64>,
    genre: Option<u16>,
    shelf: Option<i64>,
    since: Option<String>,
    lang: Option<String>,
    ext: Option<String>,
    deleted: Option<String>,
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
    group: Option<String>,
}

fn truthy(s: &Option<String>) -> bool {
    s.as_deref()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

pub async fn books(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(lib): Path<i64>,
    Query(q): Query<BooksQuery>,
    Query(rp): Query<RatingParams>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let n = [
        q.author.is_some(),
        q.series.is_some(),
        q.genre.is_some(),
        q.shelf.is_some(),
        q.since.is_some(),
    ]
    .iter()
    .filter(|x| **x)
    .count();
    if n != 1 {
        return Err(ApiError::bad_request(
            "exactly one of author, series, genre, shelf, since is required",
        ));
    }
    st.catalog(lib)?;
    let rq = rp.query()?;
    let filter = BookFilter {
        langs: split_list(q.lang.as_deref()),
        ext: q
            .ext
            .clone()
            .filter(|e| !e.is_empty())
            .map(|e| e.to_lowercase()),
        include_deleted: truthy(&q.deleted),
        q: q.q.clone().filter(|s| !s.trim().is_empty()),
    };
    let page = Page {
        cursor: q.cursor.clone().filter(|c| !c.is_empty()),
        limit: q.limit.unwrap_or(2000).clamp(1, 5000),
    };
    let shelf_keys = match q.shelf {
        Some(sid) => {
            let uid = u.id;
            Some(
                st.db
                    .run(move |c| {
                        db::get_shelf(c, uid, sid)?;
                        db::shelf_keys(c, sid, lib)
                    })
                    .await?,
            )
        }
        None => None,
    };
    let sel = if let Some(a) = q.author {
        BookSelector::Author(a)
    } else if let Some(s) = q.series {
        BookSelector::Series(s)
    } else if let Some(g) = q.genre {
        BookSelector::Genre(g)
    } else if let Some(s) = &q.since {
        let d = freelib_catalog::util::parse_date(s);
        if d.is_empty() {
            return Err(ApiError::bad_request("since must be YYYY-MM-DD"));
        }
        BookSelector::Since(d)
    } else {
        BookSelector::Ids(Vec::new())
    };
    let st2 = st.clone();
    let browse = matches!(sel, BookSelector::Author(_) | BookSelector::Series(_));
    let group = truthy(&q.group);
    let first_page = page.cursor.is_none();
    let (out, ids) = st
        .catalog_call(
            lib,
            move |cat| -> ApiResult<(serde_json::Value, Vec<i64>)> {
                let sel = match (&sel, &shelf_keys) {
                    (BookSelector::Ids(_), Some(keys)) => BookSelector::Ids(
                        cat.ids_by_keys(keys)?
                            .into_iter()
                            .map(|(_, id)| id)
                            .collect(),
                    ),
                    (s, _) => s.clone(),
                };
                let p = with_sources(&st2, u.id, lib, cat, &rq, group, |src| {
                    Ok(cat.books_page(&sel, &filter, &rq, src, &page, group)?)
                })?;
                let ids: Vec<i64> = p.books.iter().map(|b| b.id).collect();
                let books = with_marks(&st2, u.id, lib, p.books)?;
                Ok((
                    json!({"books": books, "nextCursor": p.next_cursor, "total": p.total}),
                    ids,
                ))
            },
        )
        .await?;
    if browse && first_page {
        // books of an author / series the user browses: look them up next (priority 2)
        let n = ids.len().min(crate::extrating::BROWSE_ENQUEUE);
        st.ext
            .enqueue(crate::extrating::Priority::Browse, lib, &ids[..n]);
    }
    json_etag(&headers, &out)
}

/// `GET /libraries/:lib/authors/:id/summary`: counts, series, languages, genres and top
/// co-authors of one author.
pub async fn author_summary(
    State(st): State<AppState>,
    _: Auth,
    Path((lib, id)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let v = st
        .catalog_call(lib, move |cat| -> ApiResult<_> {
            cat.author_summary(id)?
                .ok_or_else(|| ApiError::not_found("author not found"))
        })
        .await?;
    json_etag(&headers, &v)
}

/// `GET /libraries/:lib/authors/:id/coauthors`: all co-authors as compact rows
/// `[id, name, books, direct]`, ranked like the summary's `coauthors`.
pub async fn coauthors(
    State(st): State<AppState>,
    _: Auth,
    Path((lib, id)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let v = st
        .catalog_call(lib, move |cat| -> ApiResult<_> {
            if !cat.author_exists(id)? {
                return Err(ApiError::not_found("author not found"));
            }
            Ok(cat
                .coauthors(id)?
                .into_iter()
                .map(|c| (c.id, c.name, c.books, c.direct))
                .collect::<Vec<_>>())
        })
        .await?;
    json_etag(
        &headers,
        &json!({ "columns": ["id", "name", "books", "direct"], "rows": v }),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    q: Option<String>,
    kind: Option<String>,
    genre: Option<String>,
    lang: Option<String>,
    ext: Option<String>,
    from: Option<String>,
    to: Option<String>,
    limit: Option<usize>,
    deleted: Option<String>,
    group: Option<String>,
}

fn opt_date(s: &Option<String>, name: &str) -> ApiResult<Option<String>> {
    match s.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(v) => {
            let d = freelib_catalog::util::parse_date(v);
            if d.is_empty() {
                // allow a bare year
                if v.len() == 4 && v.chars().all(|c| c.is_ascii_digit()) {
                    return Ok(Some(if name == "from" {
                        format!("{v}-01-01")
                    } else {
                        format!("{v}-12-31")
                    }));
                }
                return Err(ApiError::bad_request(format!("{name} must be YYYY-MM-DD")));
            }
            Ok(Some(d))
        }
    }
}

pub async fn search(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(lib): Path<i64>,
    Query(p): Query<SearchParams>,
    Query(rp): Query<RatingParams>,
) -> ApiResult<Json<serde_json::Value>> {
    let q = p.q.clone().unwrap_or_default();
    if q.trim().chars().count() < 2 {
        return Err(ApiError::bad_request("q must have at least 2 characters"));
    }
    let kind = match p.kind.as_deref().unwrap_or("all") {
        "all" | "" => SearchKind::All,
        "books" => SearchKind::Books,
        "authors" => SearchKind::Authors,
        "series" => SearchKind::Series,
        _ => {
            return Err(ApiError::bad_request(
                "kind must be all, books, authors or series",
            ));
        }
    };
    let genres = split_list(p.genre.as_deref())
        .iter()
        .map(|g| {
            g.parse::<u16>()
                .map_err(|_| ApiError::bad_request("genre must be a list of ids"))
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let sq = SearchQuery {
        q,
        kind,
        genres,
        langs: split_list(p.lang.as_deref()),
        ext: p
            .ext
            .clone()
            .filter(|e| !e.is_empty())
            .map(|e| e.to_lowercase()),
        from: opt_date(&p.from, "from")?,
        to: opt_date(&p.to, "to")?,
        include_deleted: truthy(&p.deleted),
        limit: p.limit.unwrap_or(200).clamp(1, 1000),
        rating: rp.query()?,
        group: truthy(&p.group),
    };
    let st2 = st.clone();
    let v = st
        .catalog_call(lib, move |cat| -> ApiResult<serde_json::Value> {
            let r = with_sources(&st2, u.id, lib, cat, &sq.rating, sq.group, |src| {
                Ok(cat.search_rated(&sq, src)?)
            })?;
            let books = with_marks(&st2, u.id, lib, r.books)?;
            Ok(json!({
                "tookMs": r.took_ms,
                "authors": r.authors,
                "series": r.series,
                "books": books,
                "total": r.total,
                "facets": r.facets,
                "corrected": r.corrected,
                "didYouMean": r.did_you_mean,
                "highlight": r.highlight,
            }))
        })
        .await?;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct LangQuery {
    lib: i64,
}

pub async fn languages(
    State(st): State<AppState>,
    _: Auth,
    Query(q): Query<LangQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let rt = st.lib(q.lib)?;
    let Some(cat) = rt.handle.get() else {
        return Ok(Json(json!([])).into_response());
    };
    let v = tokio::task::spawn_blocking(move || cat.languages()).await??;
    json_etag(&headers, &v)
}
