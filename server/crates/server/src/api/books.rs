use std::path::{Path as StdPath, PathBuf};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use freelib_catalog::{BookDetail, Catalog};
use freelib_fb2conv::ConvertOptions;
use serde::Deserialize;
use std::sync::Arc;

use crate::api::browse::with_marks;
use crate::auth::Auth;
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::output::{self, Produced};
use crate::preview;
use crate::state::AppState;
use crate::util::{
    content_disposition, etag_matches, json_etag, mime_for_ext, not_modified, set_header,
};

/// Library folder of `lib`.
pub async fn lib_dir(st: &AppState, lib: i64) -> ApiResult<PathBuf> {
    let row = st
        .db
        .run(move |c| db::get_library(c, lib))
        .await?
        .ok_or_else(|| ApiError::not_found("library not found"))?;
    Ok(PathBuf::from(row.path))
}

pub async fn load_book(st: &AppState, lib: i64, id: i64) -> ApiResult<(Arc<Catalog>, BookDetail)> {
    let (_, cat) = st.catalog(lib)?;
    let c2 = cat.clone();
    let d = tokio::task::spawn_blocking(move || c2.book(id))
        .await??
        .ok_or_else(|| ApiError::not_found("book not found"))?;
    Ok((cat, d))
}

pub async fn detail(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path((lib, id)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let (_, d) = load_book(&st, lib, id).await?;
    let dir = lib_dir(&st, lib).await?;
    let info = preview::info(&st, lib, &dir, &d).await.unwrap_or_default();
    let formats = output::formats_for(&d.book.ext, st.calibre.is_some());
    let st2 = st.clone();
    let book = d.book.clone();
    let mut marked =
        tokio::task::spawn_blocking(move || with_marks(&st2, u.id, lib, vec![book])).await??;
    let b = marked
        .pop()
        .ok_or_else(|| ApiError::internal("book lost"))?;
    let mut v = serde_json::to_value(&b).map_err(|e| ApiError::internal(e.to_string()))?;
    if let Some(o) = v.as_object_mut() {
        o.insert("annotation".into(), serde_json::json!(info.annotation));
        o.insert("hasCover".into(), serde_json::json!(info.cover.is_some()));
        o.insert("file".into(), serde_json::json!(d.display_file()));
        o.insert("keywords".into(), serde_json::json!(d.keywords));
        o.insert("formats".into(), serde_json::json!(formats));
    }
    json_etag(&headers, &v)
}

#[derive(Deserialize)]
pub struct CoverQuery {
    size: Option<String>,
}

pub async fn cover(
    State(st): State<AppState>,
    _: Auth,
    Path((lib, id)): Path<(i64, i64)>,
    Query(q): Query<CoverQuery>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let thumb = match q.size.as_deref() {
        None | Some("thumb") => true,
        Some("full") => false,
        _ => return Err(ApiError::bad_request("size must be thumb or full")),
    };
    let (_, d) = load_book(&st, lib, id).await?;
    let dir = lib_dir(&st, lib).await?;
    cover_response(&st, lib, &dir, &d, thumb, Some(&headers)).await
}

/// Shared by `GET .../cover` and the OPDS cover link. Serves the real cover when the book has
/// one; for `thumb` without a real cover, a generated SVG placeholder (see `placeholder.rs`)
/// stands in, so the client never has to guess and never 404s on a plain grid render. `full`
/// without a real cover still 404s: there is no "full size placeholder" to serve.
pub async fn cover_response(
    st: &AppState,
    lib: i64,
    dir: &StdPath,
    d: &BookDetail,
    thumb: bool,
    headers: Option<&HeaderMap>,
) -> ApiResult<Response> {
    if let Some((path, mime)) = preview::cover(st, lib, dir, d, thumb).await? {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let etag = format!("\"{name}\"");
        let cache = "private, max-age=86400";
        if let Some(h) = headers
            && etag_matches(h, &etag)
        {
            return Ok(not_modified(&etag, cache));
        }
        let data = tokio::fs::read(&path).await?;
        let mut r = Response::new(Body::from(data));
        set_header(&mut r, header::CONTENT_TYPE, &mime);
        set_header(&mut r, header::ETAG, &etag);
        set_header(&mut r, header::CACHE_CONTROL, cache);
        return Ok(r);
    }
    if !thumb {
        return Err(ApiError::not_found("no cover"));
    }
    let svg = crate::placeholder::svg(&d.book.title, &d.book.authors, 160, 240);
    let etag = format!("\"ph-{}\"", crate::util::short_hash(&svg));
    let cache = "public, max-age=86400";
    if let Some(h) = headers
        && etag_matches(h, &etag)
    {
        return Ok(not_modified(&etag, cache));
    }
    let mut r = Response::new(Body::from(svg));
    set_header(&mut r, header::CONTENT_TYPE, "image/svg+xml");
    set_header(&mut r, header::ETAG, &etag);
    set_header(&mut r, header::CACHE_CONTROL, cache);
    set_header(
        &mut r,
        axum::http::HeaderName::from_static("x-cover"),
        "generated",
    );
    Ok(r)
}

#[derive(Deserialize)]
pub struct FileQuery {
    format: Option<String>,
    device: Option<i64>,
    inline: Option<String>,
}

/// Streams a produced file with `Content-Disposition`.
pub async fn file_response(
    p: Produced,
    name: &str,
    ext: &str,
    inline: bool,
) -> ApiResult<Response> {
    let mime = mime_for_ext(ext.rsplit('.').next().unwrap_or(ext));
    let mut r = match p {
        Produced::Bytes(b) => {
            let len = b.len();
            let mut r = Response::new(Body::from(b));
            set_header(&mut r, header::CONTENT_LENGTH, &len.to_string());
            r
        }
        Produced::File(path) => {
            let f = tokio::fs::File::open(&path).await?;
            let len = f.metadata().await?.len();
            let mut r = Response::new(Body::from_stream(tokio_util::io::ReaderStream::new(f)));
            set_header(&mut r, header::CONTENT_LENGTH, &len.to_string());
            r
        }
    };
    set_header(&mut r, header::CONTENT_TYPE, mime);
    set_header(
        &mut r,
        header::CONTENT_DISPOSITION,
        &content_disposition(inline, name),
    );
    set_header(&mut r, header::CACHE_CONTROL, "private, max-age=3600");
    Ok(r)
}

pub async fn file(
    State(st): State<AppState>,
    Auth(u): Auth,
    Path((lib, id)): Path<(i64, i64)>,
    Query(q): Query<FileQuery>,
) -> ApiResult<Response> {
    let (_, d) = load_book(&st, lib, id).await?;
    let (mut format, mut opts, mut template) = (
        q.format.clone().unwrap_or_else(|| "original".into()),
        ConvertOptions::default(),
        db::default_file_name(),
    );
    if let Some(dev) = q.device {
        let uid = u.id;
        let device = st.db.run(move |c| db::get_device(c, uid, dev)).await?;
        if q.format.is_none() {
            format = device.format.clone();
        }
        opts = device.options;
        template = device.file_name;
    }
    let inline = q.inline.as_deref().is_some_and(|v| v == "1" || v == "true");
    output::check_format(&st, &d.book.ext, &format)?;
    let dir = lib_dir(&st, lib).await?;
    let produced = output::produce(&st, lib, &dir, &d, &format, &opts).await?;
    let name = output::download_name(&st, &template, &d.book, &format, opts.transliterate, true);
    let ext = output::file_ext(&format, &d.book.ext);
    file_response(produced, &name, &ext, inline).await
}
