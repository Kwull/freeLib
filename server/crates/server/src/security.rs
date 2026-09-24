//! CSRF protection for state-changing API requests and common security headers.
//!
//! * `Sec-Fetch-Site: cross-site` → 403.
//! * An `Origin` header (sent by browsers on every non-GET fetch) must match `Host`
//!   (or `X-Forwarded-Host`), unless `Sec-Fetch-Site` says `same-origin` (dev proxies rewrite `Host`).
//! * A request body must be `application/json` (HTML forms cannot send that cross-site
//!   without a CORS preflight, which this server never allows).
//!
//! DNS rebinding: in open mode (and always when `FREELIB_ALLOWED_HOSTS` is set) a request whose
//! `Host` is neither `localhost`, an IP literal nor an allowed name gets 421.
//!
//! Content Security Policy: [`APP_CSP`] on every response, except book files and covers, which
//! get [`FILE_CSP`] (`sandbox`: an HTML/SVG book opened directly can run nothing on our origin).

use std::net::IpAddr;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::state::AppState;

/// Policy of the web app and the API. The reader renders EPUB sections from `blob:` URLs in a
/// same-origin iframe; blob documents inherit this policy, so book scripts cannot run.
pub const APP_CSP: &str = "default-src 'self'; script-src 'self'; object-src 'none'; \
base-uri 'none'; frame-ancestors 'self'; img-src 'self' data: blob:; \
style-src 'self' 'unsafe-inline' blob:; font-src 'self' data: blob:; connect-src 'self'; \
frame-src 'self' blob:; worker-src 'self'; form-action 'self'";

/// Policy of book files and covers.
pub const FILE_CSP: &str = "sandbox";

/// `example.org:8080` → `example.org`, `[::1]:80` → `[::1]`.
pub fn strip_port(host: &str) -> &str {
    if host.starts_with('[') {
        return match host.find(']') {
            Some(i) => &host[..=i],
            None => host,
        };
    }
    match host.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    }
}

/// Whether a request for `host` (the `Host` header, port allowed) is accepted.
pub fn host_allowed(host: &str, allowed: &[String]) -> bool {
    let h = strip_port(host.trim())
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if h == "localhost" {
        return true;
    }
    let bare = h.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<IpAddr>().is_ok() {
        return true;
    }
    allowed.iter().any(|a| {
        if let Some(suffix) = a.strip_prefix("*.") {
            h.len() > suffix.len() + 1
                && h.ends_with(suffix)
                && h.as_bytes()[h.len() - suffix.len() - 1] == b'.'
        } else {
            *a == h
        }
    })
}

/// Rejects requests for unexpected host names (DNS rebinding against open mode).
pub async fn host_guard(State(st): State<AppState>, req: Request<Body>, next: Next) -> Response {
    if st.open_mode() || !st.cfg.allowed_hosts.is_empty() {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .or_else(|| req.uri().authority().map(|a| a.to_string()));
        if let Some(h) = host
            && !host_allowed(&h, &st.cfg.allowed_hosts)
        {
            tracing::warn!(host = %h, "request for an unexpected host refused (set FREELIB_ALLOWED_HOSTS)");
            return (
                StatusCode::MISDIRECTED_REQUEST,
                "unknown host name: add it to FREELIB_ALLOWED_HOSTS",
            )
                .into_response();
        }
    }
    next.run(req).await
}

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
    h.entry(header::CONTENT_SECURITY_POLICY)
        .or_insert(header::HeaderValue::from_static(APP_CSP));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        let allowed = vec!["books.example.org".to_string(), "*.lan".to_string()];
        for ok in [
            "localhost",
            "LOCALHOST:8080",
            "127.0.0.1:8080",
            "[::1]:8080",
            "192.168.1.5",
            "books.example.org",
            "Books.Example.Org:443",
            "nas.lan",
        ] {
            assert!(host_allowed(ok, &allowed), "{ok}");
        }
        for bad in [
            "evil.com",
            "books.example.org.evil.com",
            "lan",
            "xlan",
            "localhost.evil.com",
        ] {
            assert!(!host_allowed(bad, &allowed), "{bad}");
        }
        assert_eq!(strip_port("a:1"), "a");
        assert_eq!(strip_port("[::1]:1"), "[::1]");
        assert_eq!(strip_port("::1"), "::1");
    }
}
