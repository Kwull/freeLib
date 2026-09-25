//! Password hashing, session cookies and the `Auth` / `Admin` extractors.

use std::net::{IpAddr, SocketAddr};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::HeaderMap;
use axum::http::request::Parts;

use crate::db::{self, User};
use crate::error::{ApiError, ApiResult};
use crate::state::{AppState, LimitKey};
use crate::util::sha256_hex;

pub const COOKIE: &str = "freelib_session";
const SESSION_CACHE_TTL: Duration = Duration::from_secs(60);

fn argon(fast: bool) -> Argon2<'static> {
    let params = if fast {
        Params::new(64, 1, 1, None)
    } else {
        Params::new(19 * 1024, 2, 1, None)
    };
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        params.expect("valid argon2 params"),
    )
}

pub fn hash_password(pw: &str, fast: bool) -> ApiResult<String> {
    let mut salt = [0u8; 16];
    rand::fill(&mut salt[..]);
    let salt = SaltString::encode_b64(&salt).map_err(|e| ApiError::internal(e.to_string()))?;
    argon(fast)
        .hash_password(pw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| ApiError::internal(format!("password hash: {e}")))
}

/// Verifies against a stored PHC string (parameters come from the hash).
pub fn verify_password(pw: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(h) => Argon2::default().verify_password(pw.as_bytes(), &h).is_ok(),
        Err(_) => false,
    }
}

/// Burns about the same time as a real verification when the user does not exist.
pub fn dummy_verify(pw: &str, fast: bool) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    static DUMMY_FAST: OnceLock<String> = OnceLock::new();
    let cell = if fast { &DUMMY_FAST } else { &DUMMY };
    let h = cell.get_or_init(|| hash_password("dummy-password", fast).unwrap_or_default());
    let _ = verify_password(pw, h);
}

pub fn validate_password(pw: &str) -> ApiResult<()> {
    if pw.chars().count() < 4 {
        return Err(ApiError::bad_request(
            "password must have at least 4 characters",
        ));
    }
    if pw.len() > 1024 {
        return Err(ApiError::bad_request("password too long"));
    }
    Ok(())
}

pub fn validate_username(u: &str) -> ApiResult<()> {
    let n = u.chars().count();
    if n == 0 || n > 64 || u.chars().any(|c| c.is_control() || c == ':') || u.trim() != u {
        return Err(ApiError::bad_request("invalid user name"));
    }
    Ok(())
}

pub fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        db::SESSION_DAYS * 86_400,
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_cookie() -> String {
    format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
}

pub fn is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

/// Client address for rate limiting. With `trust_proxy` (`FREELIB_TRUST_PROXY=1`, the server is
/// only reachable through one reverse proxy): `X-Real-IP` when present, else the *rightmost*
/// `X-Forwarded-For` entry, the one the proxy itself appended (entries to its left come from
/// the client and can be forged).
pub fn client_ip(parts_headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> IpAddr {
    if trust_proxy {
        let parse = |s: &str| -> Option<IpAddr> {
            let s = s.trim();
            s.parse().ok().or_else(|| {
                // "1.2.3.4:5678" / "[::1]:5678"
                s.parse::<SocketAddr>().ok().map(|a| a.ip())
            })
        };
        let real = parts_headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(parse);
        let fwd = || {
            parts_headers
                .get_all("x-forwarded-for")
                .iter()
                .filter_map(|v| v.to_str().ok())
                .flat_map(|v| v.split(','))
                .rfind(|s| !s.trim().is_empty())
                .and_then(parse)
        };
        if let Some(ip) = real.or_else(fwd) {
            return ip;
        }
    }
    peer.map(|p| p.ip())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
}

pub fn peer_addr(parts: &Parts) -> Option<SocketAddr> {
    parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0)
}

/// Current user from the session cookie (or the implicit admin in open mode).
pub async fn current_user(st: &AppState, headers: &HeaderMap) -> ApiResult<Option<User>> {
    if st.open_mode() {
        return Ok(Some(User::open_mode_admin()));
    }
    let Some(token) = cookie_value(headers, COOKIE) else {
        return Ok(None);
    };
    if token.len() > 256 {
        return Ok(None);
    }
    let key = sha256_hex(token.as_bytes());
    if let Some((u, at)) = st
        .session_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        && at.elapsed() < SESSION_CACHE_TTL
    {
        return Ok(Some(u.clone()));
    }
    let user = st.db.run(move |c| db::session_user(c, &token)).await?;
    if let Some(u) = &user {
        let mut g = st.session_cache.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() > 10_000 {
            g.clear();
        }
        g.insert(key, (u.clone(), Instant::now()));
    }
    Ok(user)
}

/// Any logged-in user.
pub struct Auth(pub User);

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, ApiError> {
        match current_user(st, &parts.headers).await? {
            Some(u) => Ok(Auth(u)),
            None => Err(ApiError::unauthorized("login required")),
        }
    }
}

/// An admin.
pub struct Admin(pub User);

impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, ApiError> {
        let Auth(u) = Auth::from_request_parts(parts, st).await?;
        if !u.is_admin() {
            return Err(ApiError::forbidden("administrator only"));
        }
        Ok(Admin(u))
    }
}

/// Verifies user name + password (login form and OPDS basic auth) with rate limiting per
/// client address and per user name ([`crate::state::RateLimiter`]); at most two Argon2
/// verifications run at once server-wide (each needs ~19 MiB).
pub async fn check_credentials(
    st: &AppState,
    ip: IpAddr,
    username: &str,
    password: &str,
) -> ApiResult<User> {
    let keys = [LimitKey::ip(ip), LimitKey::user(username)];
    if let Err(wait) = st.login_limiter.begin(&keys) {
        return Err(ApiError::rate_limited(format!(
            "too many failed logins, retry in {wait} s"
        )));
    }
    let r = verify_credentials(st, username, password).await;
    st.login_limiter.finish(&keys, matches!(r, Ok(Some(_))));
    match r {
        Ok(Some(u)) => Ok(u),
        Ok(None) => {
            tracing::info!(%ip, "failed login");
            Err(ApiError::unauthorized("invalid user name or password"))
        }
        Err(e) => Err(e),
    }
}

async fn verify_credentials(
    st: &AppState,
    username: &str,
    password: &str,
) -> ApiResult<Option<User>> {
    if username.len() > 256 || password.len() > 1024 {
        return Ok(None);
    }
    let _permit = tokio::time::timeout(Duration::from_secs(15), st.verify_sem.acquire())
        .await
        .map_err(|_| ApiError::rate_limited("server busy, retry in a few seconds"))?
        .map_err(|_| ApiError::internal("verification pool closed"))?;
    let fast = st.cfg.fast_password_hash;
    let (u, p) = (username.to_string(), password.to_string());
    let user = st.db.run(move |c| db::user_with_hash(c, &u)).await?;
    Ok(tokio::task::spawn_blocking(move || match user {
        Some((user, hash)) => verify_password(&p, &hash).then_some(user),
        None => {
            dummy_verify(&p, fast);
            None
        }
    })
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn client_ip_behind_proxy() {
        let peer: SocketAddr = "10.0.0.9:1234".parse().unwrap();
        let mut h = HeaderMap::new();
        h.insert(
            "x-forwarded-for",
            HeaderValue::from_static("6.6.6.6, 203.0.113.7"),
        );
        // not trusted: the peer
        assert_eq!(client_ip(&h, Some(peer), false), peer.ip());
        // trusted: the rightmost entry (appended by the proxy), not the forgeable leftmost one
        assert_eq!(
            client_ip(&h, Some(peer), true),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
        h.append("x-forwarded-for", HeaderValue::from_static("198.51.100.1"));
        assert_eq!(
            client_ip(&h, Some(peer), true),
            "198.51.100.1".parse::<IpAddr>().unwrap()
        );
        // X-Real-IP wins
        h.insert("x-real-ip", HeaderValue::from_static("192.0.2.5"));
        assert_eq!(
            client_ip(&h, Some(peer), true),
            "192.0.2.5".parse::<IpAddr>().unwrap()
        );
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("garbage"));
        assert_eq!(client_ip(&h, Some(peer), true), peer.ip());
    }
}
