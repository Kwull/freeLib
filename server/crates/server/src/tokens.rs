//! Personal API tokens (for the MCP endpoint): generation, hashing, scopes, bearer
//! authentication with a short cache, and per-token rate limiting.
//!
//! A token is `fl_` + 43 characters (32 random bytes, base64url). Only its SHA-256 (hex) is
//! stored; the secret is shown once when created. Scopes: `read` (catalog and own profile),
//! `write` (shelves, ratings), `send` (send to Kindle / devices).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use base64::Engine;

use crate::db::{self, ApiToken, User};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::sha256_hex;

/// Prefix of every token (recognisable in configs and secret scanners).
pub const TOKEN_PREFIX: &str = "fl_";

/// All scopes, in display order.
pub const SCOPES: [&str; 3] = ["read", "write", "send"];

const CACHE_TTL: Duration = Duration::from_secs(60);

/// A new random token secret: `fl_` + base64url(32 random bytes).
pub fn generate() -> String {
    let mut b = [0u8; 32];
    rand::fill(&mut b[..]);
    format!(
        "{TOKEN_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
    )
}

/// What is stored: SHA-256 of the secret, hex.
pub fn hash(secret: &str) -> String {
    sha256_hex(secret.as_bytes())
}

/// The displayed start of a secret (`fl_` + 8 characters).
pub fn display_prefix(secret: &str) -> String {
    secret.chars().take(TOKEN_PREFIX.len() + 8).collect()
}

/// Whether a string looks like one of our tokens (before any database lookup).
pub fn well_formed(secret: &str) -> bool {
    secret.len() == TOKEN_PREFIX.len() + 43
        && secret.starts_with(TOKEN_PREFIX)
        && secret[TOKEN_PREFIX.len()..]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// Validates a scope list: known names, at least one, deduplicated in canonical order.
pub fn clean_scopes(v: &[String]) -> ApiResult<Vec<String>> {
    for s in v {
        if !SCOPES.contains(&s.as_str()) {
            return Err(ApiError::bad_request(format!(
                "unknown scope '{s}' (use read, write, send)"
            )));
        }
    }
    let out: Vec<String> = SCOPES
        .iter()
        .filter(|s| v.iter().any(|x| x == *s))
        .map(|s| s.to_string())
        .collect();
    if out.is_empty() {
        return Err(ApiError::bad_request("choose at least one scope"));
    }
    Ok(out)
}

/// The bearer token of a request (`Authorization: Bearer fl_…`).
pub fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, rest) = v.trim().split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| rest.trim())
        .filter(|t| !t.is_empty())
}

/// An authenticated token and its user.
#[derive(Debug, Clone)]
pub struct TokenAuth {
    pub token: ApiToken,
    pub user: User,
}

impl TokenAuth {
    pub fn has(&self, scope: &str) -> bool {
        self.token.scopes.iter().any(|s| s == scope)
    }
}

/// Token cache and rate limiter.
#[derive(Default)]
pub struct Tokens {
    /// secret hash → (auth, cached at)
    cache: Mutex<HashMap<String, (TokenAuth, Instant)>>,
    /// token id → (window start, requests in the window)
    windows: Mutex<HashMap<i64, (Instant, u32)>>,
}

impl Tokens {
    /// Drops cached tokens (after a revocation or a user change).
    pub fn invalidate(&self) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Counts a request of token `id`; `Err(seconds to wait)` above `per_min` requests in the
    /// current one-minute window.
    pub fn check_rate(&self, id: i64, per_min: u32) -> Result<(), u64> {
        let mut g = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if g.len() > 10_000 {
            g.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(60));
        }
        let w = g.entry(id).or_insert((now, 0));
        if now.duration_since(w.0) >= Duration::from_secs(60) {
            *w = (now, 0);
        }
        if w.1 >= per_min {
            let wait = 60u64.saturating_sub(now.duration_since(w.0).as_secs());
            return Err(wait.max(1));
        }
        w.1 += 1;
        Ok(())
    }
}

/// Authenticates a bearer token (cached for a minute; revocations clear the cache).
pub async fn authenticate(st: &AppState, secret: &str) -> ApiResult<Option<TokenAuth>> {
    if !well_formed(secret) {
        return Ok(None);
    }
    let h = hash(secret);
    if let Some((a, at)) = st
        .tokens
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&h)
        && at.elapsed() < CACHE_TTL
    {
        return Ok(Some(a.clone()));
    }
    let h2 = h.clone();
    let found = st
        .db
        .run(move |c| {
            let r = db::token_by_hash(c, &h2)?;
            if let Some((t, _)) = &r {
                db::touch_token(c, t.id)?;
            }
            Ok(r)
        })
        .await?;
    let Some((token, user)) = found else {
        return Ok(None);
    };
    // open-mode tokens only work while the server is in open mode
    if user.id == 0 && !st.open_mode() {
        return Ok(None);
    }
    let a = TokenAuth { token, user };
    let mut g = st.tokens.cache.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() > 10_000 {
        g.clear();
    }
    g.insert(h, (a.clone(), Instant::now()));
    Ok(Some(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets() {
        let a = generate();
        let b = generate();
        assert_ne!(a, b);
        assert!(well_formed(&a), "{a}");
        assert_eq!(a.len(), 46);
        assert!(!well_formed("fl_short"));
        assert!(!well_formed(&a.replace("fl_", "xx_")));
        assert_eq!(hash(&a).len(), 64);
        assert_eq!(hash(&a), hash(&a));
        assert_ne!(hash(&a), hash(&b));
        assert_eq!(display_prefix(&a).len(), 11);
        assert!(a.starts_with(&display_prefix(&a)));
    }

    #[test]
    fn scopes() {
        let s = |v: &[&str]| clean_scopes(&v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(s(&["send", "read", "read"]).unwrap(), ["read", "send"]);
        assert!(s(&[]).is_err());
        assert!(s(&["admin"]).is_err());
    }

    #[test]
    fn bearer_header() {
        let mut h = HeaderMap::new();
        assert_eq!(bearer(&h), None);
        h.insert("authorization", "Bearer fl_x".parse().unwrap());
        assert_eq!(bearer(&h), Some("fl_x"));
        h.insert("authorization", "bearer   fl_y ".parse().unwrap());
        assert_eq!(bearer(&h), Some("fl_y"));
        h.insert("authorization", "Basic abc".parse().unwrap());
        assert_eq!(bearer(&h), None);
    }

    #[test]
    fn rate() {
        let t = Tokens::default();
        for _ in 0..3 {
            t.check_rate(1, 3).unwrap();
        }
        assert!(t.check_rate(1, 3).is_err());
        t.check_rate(2, 3).unwrap();
    }
}
