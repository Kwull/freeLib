//! CSRF protection for state-changing API requests and common security headers.
//!
//! * `Sec-Fetch-Site: cross-site` → 403.
//! * An `Origin` header (sent by browsers on every non-GET fetch) must match `Host`
//!   (or `X-Forwarded-Host`), unless `Sec-Fetch-Site` says `same-origin` (dev proxies rewrite `Host`).
//! * A request body must be `application/json` (HTML forms cannot send that cross-site
//!   without a CORS preflight, which this server never allows).

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;

fn host_of_origin(origin: &str) -> Option<&str> {
    let rest = origin.split_once("://")?.1;
    Some(rest.split('/').next().unwrap_or(rest))
}

pub fn check_csrf(req: &Request<Body>) -> Result<(), ApiError> {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return Ok(());
    }
    let h = req.headers();
    let site = h
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if site.eq_ignore_ascii_case("cross-site") {
        return Err(ApiError::forbidden("cross-site request refused"));
    }
    if !site.eq_ignore_ascii_case("same-origin")
        && let Some(origin) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        && origin != "null"
    {
        let o = host_of_origin(origin).unwrap_or("");
        let hosts = [header::HOST.as_str(), "x-forwarded-host"];
        let ok = hosts
            .iter()
            .filter_map(|n| h.get(*n).and_then(|v| v.to_str().ok()))
            .any(|host| {
                host.split(',')
                    .next()
                    .is_some_and(|x| x.trim().eq_ignore_ascii_case(o))
            });
        if !ok {
            return Err(ApiError::forbidden("origin does not match host"));
        }
    }
    let has_body = h
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(|n| n > 0)
        .unwrap_or_else(|| h.contains_key(header::TRANSFER_ENCODING));
    if has_body {
        let ct = h
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !ct
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("application/json")
        {
            return Err(ApiError::new(
                axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "bad_request",
                "request body must be application/json",
            ));
        }
    }
    Ok(())
}

pub async fn csrf_layer(req: Request<Body>, next: Next) -> Response {
    if let Err(e) = check_csrf(&req) {
        return e.into_response();
    }
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    h.entry("x-content-type-options")
        .or_insert(header::HeaderValue::from_static("nosniff"));
    r
}

/// Headers for every response (SPA and API).
pub async fn security_headers(req: Request<Body>, next: Next) -> Response {
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    h.entry("x-content-type-options")
        .or_insert(header::HeaderValue::from_static("nosniff"));
    h.entry("referrer-policy")
        .or_insert(header::HeaderValue::from_static("same-origin"));
    h.entry("x-frame-options")
        .or_insert(header::HeaderValue::from_static("SAMEORIGIN"));
    r
}
