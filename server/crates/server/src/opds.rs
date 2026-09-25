//! OPDS 1.2 catalog (`/opds/…`): navigation and acquisition feeds, OpenSearch, covers and
//! downloads; Basic auth when `opds.requireAuth`; legacy Qt `/opds_<lib>/…` redirects.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::Extension;
use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use freelib_catalog::{Book, BookFilter, BookSelector, Page, SearchKind, SearchQuery, normalize};
use freelib_fb2conv::ConvertOptions;
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use serde::Deserialize;

use crate::api::books::{file_response, lib_dir, load_book};
use crate::auth;
use crate::db::{self, OpdsConfig, User};
use crate::error::{ApiError, ApiResult};
use crate::output;
use crate::state::AppState;
use crate::util::{now_rfc3339, sha256_hex};

pub const PAGE: usize = 100;
const NAV: &str = "application/atom+xml;profile=opds-catalog;kind=navigation";
const ACQ: &str = "application/atom+xml;profile=opds-catalog;kind=acquisition";
const ATOM_CT: &str = "application/atom+xml;charset=utf-8";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/opds", get(root))
        .route("/opds/", get(root))
        .route("/opds/{lib}", get(library))
        .route("/opds/{lib}/", get(library))
        .route("/opds/{lib}/opensearch.xml", get(opensearch))
        .route("/opds/{lib}/search", get(search))
        .route("/opds/{lib}/new", get(new_books))
        .route("/opds/{lib}/authors", get(authors_root))
        .route("/opds/{lib}/authors/{prefix}", get(authors_prefix))
        .route("/opds/{lib}/author/{id}", get(author_books))
        .route("/opds/{lib}/series", get(series_root))
        .route("/opds/{lib}/series/{x}", get(series_x))
        .route("/opds/{lib}/genres", get(genres_root))
        .route("/opds/{lib}/genres/{id}", get(genre))
        .route("/opds/{lib}/book/{id}/{format}", get(book_file))
}

// ------------------------------------------------------------------ auth

fn unauthorized() -> Response {
    let mut r = (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    r.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static("Basic realm=\"freeLib\", charset=\"UTF-8\""),
    );
    r
}

#[allow(clippy::result_large_err)]
async fn basic_user(
    st: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> Result<Option<User>, Response> {
    let Some(v) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return Ok(None);
    };
    let Some(b64) = v
        .strip_prefix("Basic ")
        .or_else(|| v.strip_prefix("basic "))
    else {
        return Ok(None);
    };
    let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) else {
        return Ok(None);
    };
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let Some((user, pass)) = raw.split_once(':') else {
        return Ok(None);
    };
    let key = sha256_hex(raw.as_bytes());
    if let Some((u, at)) = st
        .basic_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        && at.elapsed() < Duration::from_secs(600)
    {
        return Ok(Some(u.clone()));
    }
    let ip = auth::client_ip(headers, peer, st.cfg.trust_proxy);
    match auth::check_credentials(st, ip, user, pass).await {
        Ok(u) => {
            let mut g = st.basic_cache.lock().unwrap_or_else(|e| e.into_inner());
            if g.len() > 1000 {
                g.clear();
            }
            g.insert(key, (u.clone(), Instant::now()));
            Ok(Some(u))
        }
        Err(e) if e.status == StatusCode::TOO_MANY_REQUESTS => Err(e.into_response()),
        Err(_) => Err(unauthorized()),
    }
}

/// OPDS gate: disabled → 404; `requireAuth` → session cookie or Basic auth.
pub async fn gate(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    req: Request,
    next: Next,
) -> Response {
    let cfg: OpdsConfig = match st.db.run(|c| db::get_setting(c, "opds")).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    if !cfg.enabled {
        return (StatusCode::NOT_FOUND, "OPDS is disabled").into_response();
    }
    if cfg.require_auth && !st.open_mode() {
        let headers = req.headers().clone();
        let cookie_user = auth::current_user(&st, &headers).await.ok().flatten();
        if cookie_user.is_none() {
            match basic_user(&st, &headers, peer.map(|p| p.0.0)).await {
                Ok(Some(_)) => {}
                Ok(None) => return unauthorized(),
                Err(r) => return r,
            }
        }
    }
    next.run(req).await
}

// ------------------------------------------------------------------ XML

struct Feed {
    w: Writer<Vec<u8>>,
}

impl Feed {
    fn new(id: &str, title: &str, self_href: &str, kind: &str, lib: Option<i64>) -> Feed {
        let mut f = Feed {
            w: Writer::new(Vec::with_capacity(16 * 1024)),
        };
        let _ =
            f.w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)));
        f.open(
            "feed",
            &[
                ("xmlns", "http://www.w3.org/2005/Atom"),
                ("xmlns:dc", "http://purl.org/dc/terms/"),
                ("xmlns:opds", "http://opds-spec.org/2010/catalog"),
                ("xmlns:opensearch", "http://a9.com/-/spec/opensearch/1.1/"),
            ],
        );
        f.text("id", id);
        f.text("title", title);
        f.text("updated", &now_rfc3339());
        f.text("icon", "/favicon.svg");
        f.open("author", &[]);
        f.text("name", "freeLib");
        f.close("author");
        f.link("self", self_href, kind, None);
        f.link("start", "/opds", NAV, None);
        if let Some(l) = lib {
            f.link(
                "search",
                &format!("/opds/{l}/opensearch.xml"),
                "application/opensearchdescription+xml",
                None,
            );
            f.link(
                "search",
                &format!("/opds/{l}/search?q={{searchTerms}}"),
                ACQ,
                None,
            );
        }
        f
    }

    fn open(&mut self, name: &str, attrs: &[(&str, &str)]) {
        let mut s = BytesStart::new(name);
        for (k, v) in attrs {
            s.push_attribute((*k, xml_clean(v).as_ref()));
        }
        let _ = self.w.write_event(Event::Start(s));
    }

    fn close(&mut self, name: &str) {
        let _ = self.w.write_event(Event::End(BytesEnd::new(name)));
    }

    fn empty(&mut self, name: &str, attrs: &[(&str, &str)]) {
        let mut s = BytesStart::new(name);
        for (k, v) in attrs {
            s.push_attribute((*k, xml_clean(v).as_ref()));
        }
        let _ = self.w.write_event(Event::Empty(s));
    }

    fn text_attrs(&mut self, name: &str, attrs: &[(&str, &str)], text: &str) {
        self.open(name, attrs);
        let _ = self
            .w
            .write_event(Event::Text(BytesText::new(&xml_clean(text))));
        self.close(name);
    }

    fn text(&mut self, name: &str, text: &str) {
        self.text_attrs(name, &[], text);
    }

    fn link(&mut self, rel: &str, href: &str, ty: &str, title: Option<&str>) {
        let mut a = vec![("rel", rel), ("href", href), ("type", ty)];
        if let Some(t) = title {
            a.push(("title", t));
        }
        self.empty("link", &a);
    }

    fn nav_entry(&mut self, id: &str, title: &str, href: &str, content: &str, target_kind: &str) {
        self.open("entry", &[]);
        self.text("title", title);
        self.text("id", id);
        self.text("updated", &now_rfc3339());
        if !content.is_empty() {
            self.text_attrs("content", &[("type", "text")], content);
        }
        self.link("subsection", href, target_kind, None);
        self.close("entry");
    }

    fn finish(mut self) -> Response {
        self.close("feed");
        let mut r = Response::new(Body::from(self.w.into_inner()));
        r.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(ATOM_CT),
        );
        r.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("private, no-cache"),
        );
        r
    }
}

/// Drops characters XML 1.0 does not allow (control characters, U+FFFE, U+FFFF) from text and
/// attribute values: catalog data comes from INPX files and may contain anything.
fn xml_clean(s: &str) -> std::borrow::Cow<'_, str> {
    let bad = |c: char| {
        !(matches!(c, '\t' | '\n' | '\r')
            || ('\u{20}'..='\u{D7FF}').contains(&c)
            || ('\u{E000}'..='\u{FFFD}').contains(&c)
            || c >= '\u{10000}')
    };
    if s.chars().any(bad) {
        std::borrow::Cow::Owned(s.chars().filter(|c| !bad(*c)).collect())
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn format_mime(f: &str, ext: &str) -> &'static str {
    match f {
        "original" => crate::util::mime_for_ext(ext),
        "epub" | "kepub" => "application/epub+zip",
        other => crate::util::mime_for_ext(other),
    }
}

fn book_entry(f: &mut Feed, lib: i64, b: &Book, calibre: bool) {
    f.open("entry", &[]);
    f.text("title", &b.title);
    f.text("id", &format!("urn:freelib:{lib}:{}", b.key));
    let updated = if b.date.len() == 10 {
        format!("{}T00:00:00Z", b.date)
    } else {
        now_rfc3339()
    };
    f.text("updated", &updated);
    for a in &b.authors {
        f.open("author", &[]);
        f.text("name", &a.name);
        f.text("uri", &format!("/opds/{lib}/author/{}", a.id));
        f.close("author");
    }
    if !b.lang.is_empty() {
        f.text("dc:language", &b.lang);
    }
    if b.date.len() >= 4 {
        f.text("dc:issued", &b.date[..4]);
    }
    let g = freelib_catalog::genres();
    for gid in &b.genres {
        if let Some(def) = g.get(*gid) {
            let term = gid.to_string();
            f.empty("category", &[("term", &term), ("label", &def.name)]);
        }
    }
    let mut summary = String::new();
    if let Some(s) = &b.series {
        summary.push_str(&s.name);
        if let Some(n) = b.serno {
            summary.push_str(&format!(" #{n}"));
        }
        summary.push_str(". ");
    }
    summary.push_str(&format!("{} · {} KB", b.ext.to_uppercase(), b.size / 1024));
    f.text_attrs("content", &[("type", "text")], &summary);
    if let Some(a) = b.authors.first() {
        f.link(
            "related",
            &format!("/opds/{lib}/author/{}", a.id),
            ACQ,
            Some(&format!("All books by {}", a.name)),
        );
    }
    if let Some(s) = &b.series {
        f.link(
            "related",
            &format!("/opds/{lib}/series/{}", s.id),
            ACQ,
            Some(&format!("Series: {}", s.name)),
        );
    }
    if matches!(b.ext.as_str(), "fb2" | "epub") {
        f.link(
            "http://opds-spec.org/image",
            &format!("/opds/{lib}/book/{}/cover", b.id),
            "image/jpeg",
            None,
        );
        f.link(
            "http://opds-spec.org/image/thumbnail",
            &format!("/opds/{lib}/book/{}/cover?size=thumb", b.id),
            "image/webp",
            None,
        );
    }
    for fmt in output::formats_for(&b.ext, calibre) {
        if !["original", "epub", "kepub", "azw3"].contains(&fmt.as_str()) {
            continue;
        }
        let label = if fmt == "original" {
            b.ext.to_uppercase()
        } else {
            fmt.to_uppercase()
        };
        f.link(
            "http://opds-spec.org/acquisition/open-access",
            &format!("/opds/{lib}/book/{}/{fmt}", b.id),
            format_mime(&fmt, &b.ext),
            Some(&label),
        );
    }
    f.close("entry");
}

// ------------------------------------------------------------------ helpers

async fn lib_name(st: &AppState, lib: i64) -> ApiResult<String> {
    Ok(st
        .db
        .run(move |c| db::get_library(c, lib))
        .await?
        .ok_or_else(|| ApiError::not_found("library not found"))?
        .name)
}

#[derive(Deserialize, Default)]
pub struct PageQuery {
    cursor: Option<String>,
    page: Option<usize>,
}

async fn books_feed(
    st: &AppState,
    lib: i64,
    sel: BookSelector,
    title: &str,
    self_base: &str,
    cursor: Option<String>,
) -> ApiResult<Response> {
    let page = Page {
        cursor: cursor.clone(),
        limit: PAGE,
    };
    let p = st
        .catalog_call(lib, move |cat| {
            Ok(cat.books(&sel, &BookFilter::default(), &page)?)
        })
        .await?;
    let self_href = match &cursor {
        Some(c) => format!("{self_base}?cursor={}", enc(c)),
        None => self_base.to_string(),
    };
    let mut f = Feed::new(
        &format!("urn:freelib:{self_base}"),
        title,
        &self_href,
        ACQ,
        Some(lib),
    );
    f.link("up", &format!("/opds/{lib}"), NAV, None);
    if let Some(n) = &p.next_cursor {
        f.link("next", &format!("{self_base}?cursor={}", enc(n)), ACQ, None);
    }
    f.text("opensearch:totalResults", &p.total.to_string());
    f.text("opensearch:itemsPerPage", &PAGE.to_string());
    let calibre = st.calibre.is_some();
    for b in &p.books {
        book_entry(&mut f, lib, b, calibre);
    }
    Ok(f.finish())
}

// ------------------------------------------------------------------ handlers

async fn root(State(st): State<AppState>) -> ApiResult<Response> {
    let libs = st.db.run(|c| db::list_libraries(c)).await?;
    if libs.len() == 1 {
        return library_feed(&st, libs[0].id, &libs[0].name, "/opds").await;
    }
    let mut f = Feed::new("urn:freelib:root", "freeLib", "/opds", NAV, None);
    for l in &libs {
        f.nav_entry(
            &format!("urn:freelib:lib:{}", l.id),
            &l.name,
            &format!("/opds/{}", l.id),
            "",
            NAV,
        );
    }
    Ok(f.finish())
}

async fn library(State(st): State<AppState>, Path(lib): Path<i64>) -> ApiResult<Response> {
    let name = lib_name(&st, lib).await?;
    library_feed(&st, lib, &name, &format!("/opds/{lib}")).await
}

async fn library_feed(st: &AppState, lib: i64, name: &str, self_href: &str) -> ApiResult<Response> {
    st.lib(lib)?;
    let mut f = Feed::new(
        &format!("urn:freelib:lib:{lib}"),
        name,
        self_href,
        NAV,
        Some(lib),
    );
    let b = format!("/opds/{lib}");
    f.open("entry", &[]);
    f.text("title", "New books");
    f.text("id", &format!("urn:freelib:lib:{lib}:new"));
    f.text("updated", &now_rfc3339());
    f.text_attrs(
        "content",
        &[("type", "text")],
        "Books added in the last 30 days",
    );
    f.link(
        "http://opds-spec.org/sort/new",
        &format!("{b}/new"),
        ACQ,
        None,
    );
    f.link("subsection", &format!("{b}/new"), ACQ, None);
    f.close("entry");
    f.nav_entry(
        &format!("urn:freelib:lib:{lib}:authors"),
        "Authors",
        &format!("{b}/authors"),
        "Browse by author",
        NAV,
    );
    f.nav_entry(
        &format!("urn:freelib:lib:{lib}:series"),
        "Series",
        &format!("{b}/series"),
        "Browse by series",
        NAV,
    );
    f.nav_entry(
        &format!("urn:freelib:lib:{lib}:genres"),
        "Genres",
        &format!("{b}/genres"),
        "Browse by genre",
        NAV,
    );
    Ok(f.finish())
}

async fn opensearch(State(st): State<AppState>, Path(lib): Path<i64>) -> ApiResult<Response> {
    let name = lib_name(&st, lib).await?;
    let mut w = Feed {
        w: Writer::new(Vec::new()),
    };
    let _ =
        w.w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)));
    w.open(
        "OpenSearchDescription",
        &[("xmlns", "http://a9.com/-/spec/opensearch/1.1/")],
    );
    w.text("ShortName", "freeLib");
    w.text("Description", &format!("Search {name}"));
    w.text("InputEncoding", "UTF-8");
    w.text("OutputEncoding", "UTF-8");
    let tpl = format!("/opds/{lib}/search?q={{searchTerms}}");
    w.empty(
        "Url",
        &[("type", "application/atom+xml"), ("template", &tpl)],
    );
    w.empty("Url", &[("type", ACQ), ("template", &tpl)]);
    w.close("OpenSearchDescription");
    let mut r = Response::new(Body::from(w.w.into_inner()));
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/opensearchdescription+xml;charset=utf-8"),
    );
    Ok(r)
}

#[derive(Deserialize)]
pub struct SearchQ {
    q: Option<String>,
    search_string: Option<String>,
    page: Option<usize>,
}

async fn search(
    State(st): State<AppState>,
    Path(lib): Path<i64>,
    Query(q): Query<SearchQ>,
) -> ApiResult<Response> {
    let text = q.q.or(q.search_string).unwrap_or_default();
    let page = q.page.unwrap_or(0).min(9);
    let qq = text.clone();
    let r = st
        .catalog_call(lib, move |cat| {
            if qq.trim().chars().count() < 2 {
                return Ok(None);
            }
            let sq = SearchQuery {
                q: qq.clone(),
                kind: SearchKind::All,
                limit: PAGE * (page + 1),
                ..Default::default()
            };
            Ok(Some(cat.search(&sq)?))
        })
        .await?;
    let self_href = format!("/opds/{lib}/search?q={}", enc(&text));
    let mut f = Feed::new(
        &format!("urn:freelib:lib:{lib}:search:{}", enc(&text)),
        &format!("Search: {text}"),
        &self_href,
        ACQ,
        Some(lib),
    );
    f.link("up", &format!("/opds/{lib}"), NAV, None);
    if let Some(r) = r {
        if page == 0 {
            for a in &r.authors {
                f.nav_entry(
                    &format!("urn:freelib:lib:{lib}:author:{}", a.id),
                    &a.name,
                    &format!("/opds/{lib}/author/{}", a.id),
                    &format!("Author · {} books", a.count),
                    ACQ,
                );
            }
            for s in &r.series {
                f.nav_entry(
                    &format!("urn:freelib:lib:{lib}:series:{}", s.id),
                    &s.name,
                    &format!("/opds/{lib}/series/{}", s.id),
                    &format!("Series · {} · {} books", s.authors, s.count),
                    ACQ,
                );
            }
        }
        let start = page * PAGE;
        if (r.books.len() as i64) < r.total && r.books.len() >= start + PAGE && page < 9 {
            f.link("next", &format!("{self_href}&page={}", page + 1), ACQ, None);
        }
        f.text("opensearch:totalResults", &r.total.to_string());
        let calibre = st.calibre.is_some();
        for b in r.books.iter().skip(start) {
            book_entry(&mut f, lib, b, calibre);
        }
    }
    Ok(f.finish())
}

async fn new_books(
    State(st): State<AppState>,
    Path(lib): Path<i64>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Response> {
    let since = st
        .catalog_call(lib, move |c2| {
            let conn = c2.conn()?;
            let max: Option<String> = conn
                .query_row("SELECT max(date) FROM book WHERE deleted=0", [], |r| {
                    r.get(0)
                })
                .ok()
                .flatten();
            let max = max
                .filter(|d| d.len() == 10)
                .unwrap_or_else(|| crate::util::date_at(crate::util::unix_now()));
            let (y, m, d) = (
                max[..4].parse().unwrap_or(1970),
                max[5..7].parse().unwrap_or(1),
                max[8..10].parse().unwrap_or(1),
            );
            let days = freelib_catalog::util::days_from_civil(y, m, d) - 30;
            Ok(crate::util::date_at(days * 86_400))
        })
        .await?;
    books_feed(
        &st,
        lib,
        BookSelector::Since(since),
        "New books",
        &format!("/opds/{lib}/new"),
        q.cursor,
    )
    .await
}

/// Rows of the cached authors/series list whose sort key starts with `prefix` (lower-case);
/// `"#"` = the non-letter group.
async fn names_with_prefix(
    st: &AppState,
    lib: i64,
    kind: &'static str,
    prefix: Option<String>,
) -> ApiResult<(Vec<(i64, String, i64)>, Vec<(String, i64)>)> {
    let rt = st.lib(lib)?;
    st.catalog_call(lib, move |cat| {
        let list = rt.name_list(cat, kind)?;
        let Some(prefix) = prefix.clone() else {
            return Ok((
                Vec::new(),
                list.letters
                    .iter()
                    .map(|(l, c, _)| (l.clone(), *c))
                    .collect(),
            ));
        };
        let first: String = prefix
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default();
        let Some((_, count, pos)) = list
            .letters
            .iter()
            .find(|(l, _, _)| *l == first || (prefix == "#" && l == "#"))
        else {
            return Ok((Vec::new(), Vec::new()));
        };
        let slice = &list.rows
            [(*pos as usize).min(list.rows.len())..((*pos + *count) as usize).min(list.rows.len())];
        let norm = normalize(&prefix);
        let rows: Vec<(i64, String, i64)> = if prefix == "#" {
            slice.to_vec()
        } else {
            slice
                .iter()
                .filter(|(_, n, _)| normalize(n).starts_with(&norm))
                .cloned()
                .collect()
        };
        Ok((rows, Vec::new()))
    })
    .await
}

async fn name_index(
    st: &AppState,
    lib: i64,
    kind: &'static str,
    prefix: Option<String>,
    page: usize,
) -> ApiResult<Response> {
    let title_kind = if kind == "authors" {
        "Authors"
    } else {
        "Series"
    };
    let base = format!("/opds/{lib}/{kind}");
    let (rows, letters) = names_with_prefix(st, lib, kind, prefix.clone()).await?;
    let self_href = match &prefix {
        Some(p) => format!("{base}/{}", enc(p)),
        None => base.clone(),
    };
    let title = match &prefix {
        Some(p) => format!("{title_kind}: {}", p.to_uppercase()),
        None => title_kind.to_string(),
    };
    let item_link = |id: i64| {
        if kind == "authors" {
            format!("/opds/{lib}/author/{id}")
        } else {
            format!("/opds/{lib}/series/{id}")
        }
    };
    let Some(prefix) = prefix else {
        let mut f = Feed::new(
            &format!("urn:freelib:lib:{lib}:{kind}"),
            &title,
            &self_href,
            NAV,
            Some(lib),
        );
        f.link("up", &format!("/opds/{lib}"), NAV, None);
        for (l, c) in letters {
            f.nav_entry(
                &format!("urn:freelib:lib:{lib}:{kind}:{}", enc(&l)),
                &l,
                &format!("{base}/{}", enc(&l.to_lowercase())),
                &format!("{c}"),
                NAV,
            );
        }
        return Ok(f.finish());
    };
    let norm = normalize(&prefix);
    // drill down while there are too many entries (but not for the numeric "#" group of series:
    // /series/<digits> is a series id)
    let can_drill = prefix != "#"
        && norm.chars().count() < 4
        && !(kind == "series" && norm.chars().all(|c| c.is_ascii_digit()));
    let mut f = Feed::new(
        &format!("urn:freelib:lib:{lib}:{kind}:{}", enc(&prefix)),
        &title,
        &self_href,
        NAV,
        Some(lib),
    );
    f.link("up", &base, NAV, None);
    if rows.len() > PAGE && can_drill {
        let n = norm.chars().count() + 1;
        let mut groups: Vec<(String, i64)> = Vec::new();
        for (_, name, _) in &rows {
            let key: String = normalize(name).chars().take(n).collect();
            match groups.last_mut() {
                Some((k, c)) if *k == key => *c += 1,
                _ => groups.push((key, 1)),
            }
        }
        // sorted keys may repeat non-adjacently after normalisation of display names; merge
        groups.sort_by(|a, b| a.0.cmp(&b.0));
        groups.dedup_by(|a, b| {
            if a.0 == b.0 {
                b.1 += a.1;
                true
            } else {
                false
            }
        });
        for (k, c) in groups {
            if c == 1 {
                if let Some((id, name, cnt)) =
                    rows.iter().find(|(_, nm, _)| normalize(nm).starts_with(&k))
                {
                    f.nav_entry(
                        &format!("urn:freelib:lib:{lib}:{kind}:id:{id}"),
                        name,
                        &item_link(*id),
                        &format!("{cnt} books"),
                        ACQ,
                    );
                }
                continue;
            }
            let label = capitalize(&k);
            f.nav_entry(
                &format!("urn:freelib:lib:{lib}:{kind}:{}", enc(&k)),
                &format!("{label}…"),
                &format!("{base}/{}", enc(&k)),
                &format!("{c}"),
                NAV,
            );
        }
        return Ok(f.finish());
    }
    let start = page * PAGE;
    if rows.len() > start + PAGE {
        f.link("next", &format!("{self_href}?page={}", page + 1), NAV, None);
    }
    for (id, name, cnt) in rows.iter().skip(start).take(PAGE) {
        f.nav_entry(
            &format!("urn:freelib:lib:{lib}:{kind}:id:{id}"),
            name,
            &item_link(*id),
            &format!("{cnt} books"),
            ACQ,
        );
    }
    Ok(f.finish())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

async fn authors_root(State(st): State<AppState>, Path(lib): Path<i64>) -> ApiResult<Response> {
    name_index(&st, lib, "authors", None, 0).await
}

async fn authors_prefix(
    State(st): State<AppState>,
    Path((lib, prefix)): Path<(i64, String)>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Response> {
    name_index(
        &st,
        lib,
        "authors",
        Some(prefix.to_lowercase()),
        q.page.unwrap_or(0),
    )
    .await
}

async fn series_root(State(st): State<AppState>, Path(lib): Path<i64>) -> ApiResult<Response> {
    name_index(&st, lib, "series", None, 0).await
}

async fn series_x(
    State(st): State<AppState>,
    Path((lib, x)): Path<(i64, String)>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Response> {
    if let Ok(id) = x.parse::<i64>() {
        let s = st
            .catalog_call(lib, move |c2| Ok(c2.series(id)?))
            .await?
            .ok_or_else(|| ApiError::not_found("series not found"))?;
        return books_feed(
            &st,
            lib,
            BookSelector::Series(id),
            &s.name,
            &format!("/opds/{lib}/series/{id}"),
            q.cursor,
        )
        .await;
    }
    name_index(
        &st,
        lib,
        "series",
        Some(x.to_lowercase()),
        q.page.unwrap_or(0),
    )
    .await
}

async fn author_books(
    State(st): State<AppState>,
    Path((lib, id)): Path<(i64, i64)>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Response> {
    let a = st
        .catalog_call(lib, move |c2| Ok(c2.author(id)?))
        .await?
        .ok_or_else(|| ApiError::not_found("author not found"))?;
    books_feed(
        &st,
        lib,
        BookSelector::Author(id),
        &a.name,
        &format!("/opds/{lib}/author/{id}"),
        q.cursor,
    )
    .await
}

async fn genres_root(
    State(st): State<AppState>,
    Path(lib): Path<i64>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    genre_nav(&st, lib, 0, crate::util::preferred_lang(&headers)).await
}

async fn genre_nav(
    st: &AppState,
    lib: i64,
    parent: u16,
    lang: &'static str,
) -> ApiResult<Response> {
    let counts = st
        .catalog_call(lib, move |cat| Ok(cat.genres(lang)?))
        .await?;
    let g = freelib_catalog::genres();
    let title = if parent == 0 {
        "Genres".to_string()
    } else {
        g.get(parent)
            .map(|d| d.localized(lang).to_string())
            .unwrap_or_default()
    };
    let self_href = if parent == 0 {
        format!("/opds/{lib}/genres")
    } else {
        format!("/opds/{lib}/genres/{parent}")
    };
    let mut f = Feed::new(
        &format!("urn:freelib:lib:{lib}:genres:{parent}"),
        &title,
        &self_href,
        NAV,
        Some(lib),
    );
    f.link(
        "up",
        &if parent == 0 {
            format!("/opds/{lib}")
        } else {
            format!("/opds/{lib}/genres")
        },
        NAV,
        None,
    );
    for gc in counts.iter().filter(|c| c.parent == parent && c.count > 0) {
        let has_children = !g.children(gc.id).is_empty();
        let kind = if has_children { NAV } else { ACQ };
        f.nav_entry(
            &format!("urn:freelib:lib:{lib}:genre:{}", gc.id),
            &gc.name,
            &format!("/opds/{lib}/genres/{}", gc.id),
            &format!("{} books", gc.count),
            kind,
        );
    }
    Ok(f.finish())
}

#[derive(Deserialize)]
pub struct GenreQ {
    cursor: Option<String>,
    all: Option<String>,
}

async fn genre(
    State(st): State<AppState>,
    Path((lib, id)): Path<(i64, u16)>,
    Query(q): Query<GenreQ>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let lang = crate::util::preferred_lang(&headers);
    let g = freelib_catalog::genres();
    let def = g
        .get(id)
        .ok_or_else(|| ApiError::not_found("genre not found"))?;
    if !g.children(id).is_empty() && q.all.is_none() {
        return genre_nav(&st, lib, id, lang).await;
    }
    books_feed(
        &st,
        lib,
        BookSelector::Genre(id),
        def.localized(lang),
        &format!("/opds/{lib}/genres/{id}"),
        q.cursor,
    )
    .await
}

#[derive(Deserialize)]
pub struct FileQ {
    size: Option<String>,
}

async fn book_file(
    State(st): State<AppState>,
    Path((lib, id, format)): Path<(i64, i64, String)>,
    Query(q): Query<FileQ>,
) -> ApiResult<Response> {
    let (_, d) = load_book(&st, lib, id).await?;
    let dir = lib_dir(&st, lib).await?;
    if format == "cover" {
        let thumb = q.size.as_deref() == Some("thumb");
        return crate::api::books::cover_response(&st, lib, &dir, &d, thumb, None).await;
    }
    let opts = ConvertOptions::default();
    let produced = output::produce(&st, lib, &dir, &d, &format, &opts, None).await?;
    let name = output::download_name(&st, &db::default_file_name(), &d.book, &format, false, true);
    file_response(
        produced,
        &name,
        &output::file_ext(&format, &d.book.ext),
        false,
    )
    .await
}

/// `/opds_<lib>/…` (Qt desktop app URLs) → the new paths.
pub fn legacy_redirect(path: &str, query: Option<&str>) -> Option<Response> {
    let rest = path.strip_prefix("/opds_")?;
    let (lib, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let lib: i64 = lib.parse().ok()?;
    let parts: Vec<&str> = tail.split('/').filter(|s| !s.is_empty()).collect();
    let base = format!("/opds/{lib}");
    let target = match parts.as_slice() {
        [] => base,
        ["authorsindex"] => format!("{base}/authors"),
        ["authorsindex", p, ..] => format!("{base}/authors/{p}"),
        ["author", id, ..] | ["authorbooks", id, ..] => format!("{base}/author/{id}"),
        ["sequencesindex"] => format!("{base}/series"),
        ["sequencesindex", p, ..] => format!("{base}/series/{p}"),
        ["sequencebooks", id, ..] => format!("{base}/series/{id}"),
        ["genres"] => format!("{base}/genres"),
        ["genres", id, ..] => format!("{base}/genres/{id}"),
        ["opensearch.xml"] => format!("{base}/opensearch.xml"),
        ["search"] => {
            let q = query
                .and_then(|q| {
                    q.split('&').find_map(|kv| {
                        kv.strip_prefix("search_string=")
                            .or_else(|| kv.strip_prefix("q="))
                    })
                })
                .unwrap_or("");
            format!("{base}/search?q={q}")
        }
        ["book", id, fmt] => format!("{base}/book/{id}/{fmt}"),
        _ => base,
    };
    let mut r = StatusCode::MOVED_PERMANENTLY.into_response();
    crate::util::set_header(&mut r, header::LOCATION, &target);
    Some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_clean_strips_invalid() {
        assert_eq!(xml_clean("a\u{1}b\u{FFFE}c\td"), "abc\td");
        assert!(matches!(xml_clean("plain"), std::borrow::Cow::Borrowed(_)));
        let mut f = Feed::new("id", "t\u{8}", "/opds", NAV, None);
        f.link("self", "/x\u{1b}y", NAV, Some("ti\u{0}tle"));
        let body = String::from_utf8(f.w.into_inner()).unwrap();
        assert!(!body.chars().any(|c| (c as u32) < 0x20 && c != '\n'));
        assert!(body.contains("href=\"/xy\""));
    }
}
