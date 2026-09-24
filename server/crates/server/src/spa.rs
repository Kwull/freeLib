//! The web app: embedded `web/dist` (or `FREELIB_WEB_DIR`), `/assets/*` with long cache,
//! every other GET → `index.html`.

use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

use crate::state::AppState;
use crate::util::{etag_matches, not_modified, set_header};

#[derive(RustEmbed)]
#[folder = "$FREELIB_WEB_DIST"]
struct Dist;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

fn load(dir: Option<&Path>, path: &str) -> Option<(Vec<u8>, String)> {
    match dir {
        Some(d) => {
            let rel = PathBuf::from(path);
            if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
                return None;
            }
            let data = std::fs::read(d.join(rel)).ok()?;
            let etag = format!("\"{}\"", &crate::util::sha256_hex(&data)[..20]);
            Some((data, etag))
        }
        None => {
            let f = Dist::get(path)?;
            let etag = format!("\"{}\"", &hex::encode(f.metadata.sha256_hash())[..20]);
            Some((f.data.into_owned(), etag))
        }
    }
}

fn respond(headers: &HeaderMap, path: &str, data: Vec<u8>, etag: &str, cache: &str) -> Response {
    if etag_matches(headers, etag) {
        return not_modified(etag, cache);
    }
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut r = Response::new(Body::from(data));
    set_header(&mut r, header::CONTENT_TYPE, mime.as_ref());
    set_header(&mut r, header::ETAG, etag);
    set_header(&mut r, header::CACHE_CONTROL, cache);
    r
}

pub async fn serve(State(st): State<AppState>, method: Method, uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") || path == "/api" {
        return crate::error::ApiError::not_found("no such endpoint").into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let dir = st.cfg.web_dir.as_deref();
    let rel = path.trim_start_matches('/');
    if !rel.is_empty()
        && let Some((data, etag)) = load(dir, rel)
    {
        let cache = if rel.starts_with("assets/") { IMMUTABLE } else { REVALIDATE };
        return respond(&headers, rel, data, &etag, cache);
    }
    if rel.starts_with("assets/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    match load(dir, "index.html") {
        Some((data, etag)) => respond(&headers, "index.html", data, &etag, REVALIDATE),
        None => (StatusCode::NOT_FOUND, "web app not built (web/dist missing)").into_response(),
    }
}
