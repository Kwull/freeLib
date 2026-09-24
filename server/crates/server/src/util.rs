//! Small helpers: time, hashing, HTTP caching headers, file names and paths.

use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use freelib_catalog::util::civil_from_days;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::ApiError;

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// RFC 3339 (`2024-05-01T12:34:56Z`) for a Unix time.
pub fn rfc3339_at(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

pub fn now_rfc3339() -> String {
    rfc3339_at(unix_now())
}

/// `YYYY-MM-DD` of a Unix time.
pub fn date_at(secs: i64) -> String {
    rfc3339_at(secs)[..10].to_string()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Short stable hash for cache file names.
pub fn short_hash(data: &[u8]) -> String {
    sha256_hex(data)[..24].to_string()
}

pub fn random_token(bytes: usize) -> String {
    use base64::Engine;
    let mut buf = vec![0u8; bytes];
    rand::fill(&mut buf[..]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

pub fn random_id() -> String {
    let mut buf = [0u8; 8];
    rand::fill(&mut buf[..]);
    hex::encode(buf)
}

/// True when `If-None-Match` matches `etag` (or `*`).
pub fn etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    let Some(v) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let bare = etag.trim_start_matches("W/");
    v.split(',')
        .map(str::trim)
        .any(|t| t == "*" || t.trim_start_matches("W/") == bare)
}

pub fn not_modified(etag: &str, cache_control: &str) -> Response {
    let mut r = StatusCode::NOT_MODIFIED.into_response();
    set_header(&mut r, header::ETAG, etag);
    set_header(&mut r, header::CACHE_CONTROL, cache_control);
    r
}

pub fn set_header(r: &mut Response, name: header::HeaderName, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        r.headers_mut().insert(name, v);
    }
}

/// JSON response with a content-hash ETag and 304 support (`private, no-cache`).
pub fn json_etag<T: Serialize>(headers: &HeaderMap, value: &T) -> Result<Response, ApiError> {
    let body = serde_json::to_vec(value).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(bytes_etag(
        headers,
        body,
        "application/json",
        "private, no-cache",
    ))
}

pub fn bytes_etag(
    headers: &HeaderMap,
    body: Vec<u8>,
    content_type: &str,
    cache_control: &str,
) -> Response {
    let etag = format!("\"{}\"", &sha256_hex(&body)[..20]);
    if etag_matches(headers, &etag) {
        return not_modified(&etag, cache_control);
    }
    let mut r = Response::new(Body::from(body));
    set_header(&mut r, header::CONTENT_TYPE, content_type);
    set_header(&mut r, header::ETAG, &etag);
    set_header(&mut r, header::CACHE_CONTROL, cache_control);
    r
}

/// `Content-Disposition` with an ASCII fallback and an RFC 5987 UTF-8 `filename*`.
pub fn content_disposition(inline: bool, name: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    let fallback: String = freelib_fb2conv::transliteration(name)
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' && c != '%' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let fallback = if fallback.trim().is_empty() {
        "book".to_string()
    } else {
        fallback
    };
    format!(
        "{}; filename=\"{}\"; filename*=UTF-8''{}",
        if inline { "inline" } else { "attachment" },
        fallback,
        percent_encoding::utf8_percent_encode(name, SET)
    )
}

/// Resolves a user-supplied path (absolute, or relative to `root`) and checks that the result
/// stays inside `root` after resolving symlinks. The path must exist.
pub fn resolve_inside(root: &Path, user_path: &str) -> Result<PathBuf, ApiError> {
    let root_c = root
        .canonicalize()
        .map_err(|_| ApiError::bad_request("books folder is not available"))?;
    let p = Path::new(user_path.trim());
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root_c.join(p)
    };
    if joined
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(ApiError::forbidden("path must not contain '..'"));
    }
    let c = joined
        .canonicalize()
        .map_err(|_| ApiError::bad_request(format!("path not found: {user_path}")))?;
    if !c.starts_with(&root_c) {
        return Err(ApiError::forbidden("path is outside the books folder"));
    }
    Ok(c)
}

/// Path of `p` relative to `root` (both canonical), `""` for the root itself.
pub fn relative_to(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

/// Sanitises a relative sub-folder (device target): no `..`, no absolute paths, no empty parts.
pub fn safe_subdir(s: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in s.replace('\\', "/").split('/') {
        let part = part.trim();
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.chars().any(|c| c.is_control() || c == ':') {
            return None;
        }
        out.push(part);
    }
    Some(out)
}

/// Joins `rel` (as produced by `fb2conv::file_name`, may contain `/`) below `dir`, refusing escapes.
pub fn join_safe(dir: &Path, rel: &str) -> Option<PathBuf> {
    let sub = safe_subdir(rel)?;
    if sub.as_os_str().is_empty() {
        return None;
    }
    Some(dir.join(sub))
}

/// Parses `Accept-Encoding`: returns "br", "gzip" or "" (identity).
pub fn preferred_encoding(headers: &HeaderMap) -> &'static str {
    let Some(v) = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
    else {
        return "";
    };
    let mut br = false;
    let mut gz = false;
    for part in v.split(',') {
        let mut it = part.split(';');
        let name = it.next().unwrap_or("").trim().to_ascii_lowercase();
        let q = it
            .find_map(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .and_then(|q| q.parse::<f32>().ok())
            })
            .unwrap_or(1.0);
        if q <= 0.0 {
            continue;
        }
        match name.as_str() {
            "br" => br = true,
            "gzip" | "x-gzip" => gz = true,
            _ => {}
        }
    }
    if br {
        "br"
    } else if gz {
        "gzip"
    } else {
        ""
    }
}

pub fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "fb2" => "application/x-fictionbook+xml",
        "epub" | "kepub" => "application/epub+zip",
        "azw3" => "application/vnd.amazon.ebook",
        "mobi" => "application/x-mobipocket-ebook",
        "pdf" => "application/pdf",
        "djvu" => "image/vnd.djvu",
        "txt" => "text/plain; charset=utf-8",
        "rtf" => "application/rtf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "html" | "htm" => "text/html; charset=utf-8",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposition() {
        let d = content_disposition(false, "Стругацкий А. - Пикник.epub");
        assert!(d.starts_with("attachment; filename=\""));
        assert!(d.contains("filename*=UTF-8''%D0%A1"));
        assert!(!d.contains("\"S\""));
    }

    #[test]
    fn subdirs() {
        assert_eq!(safe_subdir("a/b").unwrap(), PathBuf::from("a/b"));
        assert!(safe_subdir("../x").is_none());
        assert_eq!(safe_subdir("/abs/x").unwrap(), PathBuf::from("abs/x"));
        assert!(join_safe(Path::new("/e"), "").is_none());
    }

    #[test]
    fn encodings() {
        let mut h = HeaderMap::new();
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br;q=0"),
        );
        assert_eq!(preferred_encoding(&h), "gzip");
        h.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, br"),
        );
        assert_eq!(preferred_encoding(&h), "br");
    }

    #[test]
    fn time() {
        assert_eq!(rfc3339_at(0), "1970-01-01T00:00:00Z");
        assert_eq!(date_at(86_400 * 365), "1971-01-01");
    }
}
