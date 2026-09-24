use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use freelib_catalog::{Book, BookFilter, BookSelector, Catalog, Page, SearchKind, SearchQuery};
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

/// API `Book` (catalog book + the user's rating and shelves).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookOut {
    #[serde(flatten)]
    pub book: Book,
    pub rating: i64,
    pub shelves: Vec<i64>,
}

/// Adds rating and shelves of `user_id` (blocking).
pub fn with_marks(st: &AppState, user_id: i64, lib: i64, books: Vec<Book>) -> ApiResult<Vec<BookOut>> {
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
            book: b,
        })
        .collect())
}

#[derive(Deserialize)]
pub struct VersionQuery {
    v: Option<i64>,
}

/// Builds (or returns the cached) authors / series list body. Blocking.
pub fn build_list(rt: &LibRuntime, cat: &Catalog, kind: &'static str) -> ApiResult<Arc<CachedBody>> {
    let version = cat.catalog_version();
    if let Some(b) = rt.cached_list(kind, version) {
        return Ok(b);
    }
    let list = if kind == "authors" { cat.authors()? } else { cat.series_list()? };
    let json = serde_json::to_vec(&json!({
        "version": version,
        "columns": ["id", "name", "count"],
        "rows": list.rows,
        "letters": list.letters,
    }))
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let body = Arc::new(CachedBody::new(version, format!("W/\"{kind}-{}-{version}\"", rt.id), json));
    rt.put_list(kind, body.clone());
    Ok(body)
}

async fn name_list(st: AppState, lib: i64, kind: &'static str, v: Option<i64>, headers: HeaderMap) -> ApiResult<Response> {
    let rt = st.lib(lib)?;
    let Some(cat) = rt.handle.get() else {
        return Ok(Json(json!({"version": 0, "columns": ["id", "name", "count"], "rows": [], "letters": []})).into_response());
    };
    let version = cat.catalog_version();
    let cache = if v == Some(version) { IMMUTABLE } else { REVALIDATE };
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

pub async fn genres(
    State(st): State<AppState>,
    _: Auth,
    Path(lib): Path<i64>,
    Query(q): Query<VersionQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let rt = st.lib(lib)?;
    let cat = rt.handle.get();
    let version = cat.as_ref().map(|c| c.catalog_version()).unwrap_or(0);
    let cache = if q.v == Some(version) && version > 0 { IMMUTABLE } else { REVALIDATE };
    let etag = format!("W/\"genres-{lib}-{version}\"");
    if etag_matches(&headers, &etag) {
        return Ok(not_modified(&etag, cache));
    }
    let list = tokio::task::spawn_blocking(move || -> ApiResult<serde_json::Value> {
        match cat {
            Some(c) => Ok(serde_json::to_value(c.genres()?).unwrap_or_default()),
            None => Ok(serde_json::to_value(
                freelib_catalog::genres()
                    .all()
                    .iter()
                    .map(|g| json!({"id": g.id, "name": g.name, "parent": g.parent, "count": 0}))
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
pub struct BooksQuery {
    author: Option<i64>,
    series: Option<i64>,
    genre: Option<u16>,
    shelf: Option<i64>,
    since: Option<String>,
    lang: Option<String>,
    ext: Option<String>,
    deleted: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

fn truthy(s: &Option<String>) -> bool {
    s.as_deref().is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

pub async fn books(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path(lib): Path<i64>,
    Query(q): Query<BooksQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let n = [q.author.is_some(), q.series.is_some(), q.genre.is_some(), q.shelf.is_some(), q.since.is_some()]
        .iter()
        .filter(|x| **x)
        .count();
    if n != 1 {
        return Err(ApiError::bad_request("exactly one of author, series, genre, shelf, since is required"));
    }
    let (_, cat) = st.catalog(lib)?;
    let filter = BookFilter {
        langs: split_list(q.lang.as_deref()),
        ext: q.ext.clone().filter(|e| !e.is_empty()).map(|e| e.to_lowercase()),
        include_deleted: truthy(&q.deleted),
    };
    let page = Page { cursor: q.cursor.clone().filter(|c| !c.is_empty()), limit: q.limit.unwrap_or(2000).clamp(1, 5000) };
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
    let out = tokio::task::spawn_blocking(move || -> ApiResult<serde_json::Value> {
        let sel = match (sel, shelf_keys) {
            (BookSelector::Ids(_), Some(keys)) => {
                BookSelector::Ids(cat.ids_by_keys(&keys)?.into_iter().map(|(_, id)| id).collect())
            }
            (s, _) => s,
        };
        let p = cat.books(&sel, &filter, &page)?;
        let books = with_marks(&st2, u.id, lib, p.books)?;
        Ok(json!({"books": books, "nextCursor": p.next_cursor, "total": p.total}))
    })
    .await??;
    json_etag(&headers, &out)
}

#[derive(Deserialize)]
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
}

fn opt_date(s: &Option<String>, name: &str) -> ApiResult<Option<String>> {
    match s.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(v) => {
            let d = freelib_catalog::util::parse_date(v);
            if d.is_empty() {
                // allow a bare year
                if v.len() == 4 && v.chars().all(|c| c.is_ascii_digit()) {
                    return Ok(Some(if name == "from" { format!("{v}-01-01") } else { format!("{v}-12-31") }));
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
        _ => return Err(ApiError::bad_request("kind must be all, books, authors or series")),
    };
    let genres = split_list(p.genre.as_deref())
        .iter()
        .map(|g| g.parse::<u16>().map_err(|_| ApiError::bad_request("genre must be a list of ids")))
        .collect::<ApiResult<Vec<_>>>()?;
    let sq = SearchQuery {
        q,
        kind,
        genres,
        langs: split_list(p.lang.as_deref()),
        ext: p.ext.clone().filter(|e| !e.is_empty()).map(|e| e.to_lowercase()),
        from: opt_date(&p.from, "from")?,
        to: opt_date(&p.to, "to")?,
        include_deleted: truthy(&p.deleted),
        limit: p.limit.unwrap_or(200).clamp(1, 1000),
    };
    let (_, cat) = st.catalog(lib)?;
    let st2 = st.clone();
    let v = tokio::task::spawn_blocking(move || -> ApiResult<serde_json::Value> {
        let r = cat.search(&sq)?;
        let books = with_marks(&st2, u.id, lib, r.books)?;
        Ok(json!({
            "tookMs": r.took_ms,
            "authors": r.authors,
            "series": r.series,
            "books": books,
            "total": r.total,
            "facets": r.facets,
        }))
    })
    .await??;
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct LangQuery {
    lib: i64,
}

pub async fn languages(State(st): State<AppState>, _: Auth, Query(q): Query<LangQuery>, headers: HeaderMap) -> ApiResult<Response> {
    let rt = st.lib(q.lib)?;
    let Some(cat) = rt.handle.get() else { return Ok(Json(json!([])).into_response()) };
    let v = tokio::task::spawn_blocking(move || cat.languages()).await??;
    json_etag(&headers, &v)
}
