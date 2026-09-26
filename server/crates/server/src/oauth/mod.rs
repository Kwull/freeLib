//! OAuth 2.1 authorization server for the MCP endpoint, following the MCP authorization
//! specification (2025-11-25 / 2026-07-28), so that claude.ai, Claude Desktop, Claude Code and
//! other MCP clients can connect with a browser sign-in instead of a pasted token.
//!
//! * **Discovery**: Protected Resource Metadata (RFC 9728) at
//!   `/.well-known/oauth-protected-resource[/mcp]`, Authorization Server Metadata (RFC 8414) at
//!   `/.well-known/oauth-authorization-server`; `/mcp` answers 401 with
//!   `WWW-Authenticate: Bearer resource_metadata="…"`.
//! * **Clients**: Client ID Metadata Documents ([`cimd`], what Claude uses) and Dynamic Client
//!   Registration (RFC 7591, `/oauth/register`). Public clients only (PKCE, no secrets).
//!   Redirect URIs: `https://` on a trusted host (`FREELIB_OAUTH_CLIENT_HOSTS`, default
//!   claude.ai and claude.com) or loopback `http://127.0.0.1|[::1]|localhost` (RFC 8252; the
//!   port is ignored when matching). Exact matching otherwise.
//! * **Authorization**: `/oauth/authorize` checks the client and redirect URI (errors about
//!   those are shown, never redirected), then PKCE S256, scopes and the resource (errors are
//!   redirected with `state` and `iss`), and sends the browser to the web app's consent page
//!   (`/oauth/consent?request=…`); the user signs in there as usual (password or single
//!   sign-on). The page reads and answers the request through `/api/v1/oauth/requests/{id}`
//!   (session cookie, the API's CSRF checks and a per-request token).
//! * **Tokens** (`/oauth/token`): opaque, only their SHA-256 is stored; access tokens
//!   (`flo_…`) last an hour and are bound to the resource `<FREELIB_PUBLIC_URL>/mcp`; refresh
//!   tokens (`flr_…`) last 90 days, are rotated on every use, and a reused (already rotated)
//!   refresh token or authorization code revokes the whole grant. Revocation: RFC 7009
//!   (`/oauth/revoke`) and Settings → Account.
//! * **Limits**: per-address rate limits on register / authorize / token, at most
//!   [`MAX_CLIENTS`] registered clients (never-used ones expire after a day), pending requests
//!   and codes in memory with short lifetimes.

// handlers return ready OAuth error responses as their `Err`
#![allow(clippy::result_large_err)]

pub mod cimd;
pub mod store;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{ConnectInfo, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::db::{self, User};
use crate::error::ApiResult;
use crate::state::{AppState, LimitKey};
use crate::tokens::SCOPES;
use crate::util::{random_token, sha256_hex, unix_now};

pub use openidconnect::url::Url;

/// Access token lifetime.
pub const ACCESS_TTL: i64 = 3600;
/// Refresh token lifetime (renewed by every rotation).
pub const REFRESH_TTL: i64 = 90 * 86_400;
/// Authorization code lifetime.
pub const CODE_TTL: Duration = Duration::from_secs(300);
/// Lifetime of an authorization request waiting for the user's consent.
pub const PENDING_TTL: Duration = Duration::from_secs(900);
/// Registered (DCR) clients kept at most.
pub const MAX_CLIENTS: i64 = 500;
/// A registered client never used for an authorization is removed after this long.
pub const UNUSED_CLIENT_SECS: i64 = 86_400;
/// A registered client without grants is removed this long after its last use.
pub const IDLE_CLIENT_SECS: i64 = 90 * 86_400;
pub const ACCESS_PREFIX: &str = "flo_";
pub const REFRESH_PREFIX: &str = "flr_";
const MAX_PENDING: usize = 10_000;
const MAX_PENDING_PER_ADDR: usize = 30;
const MAX_CODES: usize = 10_000;

/// The issuer (`FREELIB_PUBLIC_URL`) when OAuth is available: an `https://` URL, or `http://`
/// on a loopback host (development). `None` disables the authorization server.
pub fn issuer(st: &AppState) -> Option<String> {
    let u = st.cfg.public_url.as_deref()?;
    let p = Url::parse(u).ok()?;
    let ok = match p.scheme() {
        "https" => true,
        "http" => matches!(
            p.host_str(),
            Some("localhost") | Some("127.0.0.1") | Some("[::1]")
        ),
        _ => false,
    };
    (ok && p.host_str().is_some() && p.query().is_none() && matches!(p.path(), "" | "/"))
        .then(|| u.trim_end_matches('/').to_string())
}

/// The canonical resource identifier of the MCP endpoint (`<issuer>/mcp`).
pub fn resource(issuer: &str) -> String {
    format!("{issuer}/mcp")
}

/// The Protected Resource Metadata URL announced in `WWW-Authenticate`.
pub fn resource_metadata_url(issuer: &str) -> String {
    format!("{issuer}/.well-known/oauth-protected-resource/mcp")
}

/// Whether a bearer secret looks like one of our OAuth access tokens.
pub fn is_access_token(s: &str) -> bool {
    well_formed(s, ACCESS_PREFIX)
}

fn well_formed(s: &str, prefix: &str) -> bool {
    s.len() == prefix.len() + 43
        && s.starts_with(prefix)
        && s[prefix.len()..]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn new_secret(prefix: &str) -> String {
    format!("{prefix}{}", random_token(32))
}

// ---------------------------------------------------------------- policy (pure functions)

fn is_loopback_host(h: &str) -> bool {
    matches!(h, "localhost" | "127.0.0.1" | "[::1]")
}

/// Whether `host` (a domain name) is trusted by `trusted` (`*`, exact names, `*.suffix`).
pub fn host_trusted(host: &str, trusted: &[String]) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h.parse::<std::net::IpAddr>().is_ok() || h.starts_with('[') {
        return false;
    }
    trusted.iter().any(|t| {
        if t == "*" {
            true
        } else if let Some(suffix) = t.strip_prefix("*.") {
            h.len() > suffix.len() + 1
                && h.ends_with(suffix)
                && h.as_bytes()[h.len() - suffix.len() - 1] == b'.'
        } else {
            *t == h
        }
    })
}

/// Checks a redirect URI a client registers or declares: `https://` on a trusted host, or
/// `http://` on a loopback host; no fragment, no user info, at most 512 characters.
pub fn check_redirect_uri(uri: &str, trusted: &[String]) -> Result<Url, String> {
    if uri.len() > 512 || uri.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("invalid redirect URI".into());
    }
    let u = Url::parse(uri).map_err(|_| format!("{uri} is not an absolute URL"))?;
    if u.fragment().is_some() {
        return Err(format!("{uri} has a fragment"));
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err(format!("{uri} has user info"));
    }
    let host = u.host_str().unwrap_or("");
    match u.scheme() {
        "http" if is_loopback_host(host) => Ok(u),
        "https" if host_trusted(host, trusted) => Ok(u),
        "https" => Err(format!(
            "redirect host {host} is not trusted (FREELIB_OAUTH_CLIENT_HOSTS)"
        )),
        _ => Err(format!(
            "{uri}: redirect URIs must be https:// on a trusted host or http:// on a loopback address"
        )),
    }
}

/// Whether a requested redirect URI matches a registered one: exactly, or for loopback
/// `http://` URIs with the port ignored (RFC 8252 section 7.3).
pub fn redirect_matches(registered: &str, requested: &str) -> bool {
    if registered == requested {
        return true;
    }
    let (Ok(a), Ok(b)) = (Url::parse(registered), Url::parse(requested)) else {
        return false;
    };
    a.scheme() == "http"
        && b.scheme() == "http"
        && a.host_str().is_some_and(is_loopback_host)
        && a.host_str() == b.host_str()
        && a.path() == b.path()
        && a.query() == b.query()
        && a.fragment().is_none()
        && b.fragment().is_none()
        && a.username().is_empty()
        && b.username().is_empty()
        && a.password().is_none()
        && b.password().is_none()
}

/// The scopes of a `scope` parameter: known names in canonical order; `offline_access` is
/// accepted and ignored (refresh tokens are always issued); none means all.
pub fn parse_scopes(s: Option<&str>) -> Result<Vec<String>, String> {
    let words: Vec<&str> = s.unwrap_or("").split_whitespace().collect();
    for w in &words {
        if *w != "offline_access" && !SCOPES.contains(w) {
            return Err(format!("unknown scope {w}"));
        }
    }
    let out: Vec<String> = SCOPES
        .iter()
        .filter(|x| words.contains(x))
        .map(|x| x.to_string())
        .collect();
    Ok(if out.is_empty() {
        SCOPES.iter().map(|s| s.to_string()).collect()
    } else {
        out
    })
}

fn norm_uri(s: &str) -> Option<String> {
    let u = Url::parse(s).ok()?;
    if u.fragment().is_some() {
        return None;
    }
    Some(u.as_str().trim_end_matches('/').to_string())
}

/// Checks an RFC 8707 `resource` against this server: `<issuer>/mcp` or the issuer itself
/// (case of scheme and host and a trailing slash do not matter). Returns the canonical
/// resource the token is bound to, `None` for another resource (`invalid_target`).
pub fn check_resource(issuer: &str, requested: Option<&str>) -> Option<String> {
    let canonical = resource(issuer);
    let Some(r) = requested.filter(|r| !r.is_empty()) else {
        return Some(canonical);
    };
    let n = norm_uri(r)?;
    (Some(&n) == norm_uri(&canonical).as_ref() || Some(&n) == norm_uri(issuer).as_ref())
        .then_some(canonical)
}

fn pkce_chars_ok(s: &str) -> bool {
    (43..=128).contains(&s.len())
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~'))
}

/// PKCE S256: `BASE64URL(SHA256(verifier)) == challenge` (constant time).
pub fn pkce_ok(verifier: &str, challenge: &str) -> bool {
    if !pkce_chars_ok(verifier) {
        return false;
    }
    let h = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    bool::from(subtle::ConstantTimeEq::ct_eq(
        h.as_bytes(),
        challenge.as_bytes(),
    ))
}

/// `uri` with query parameters appended.
fn with_params(uri: &str, params: &[(&str, &str)]) -> String {
    match Url::parse(uri) {
        Ok(mut u) => {
            {
                let mut q = u.query_pairs_mut();
                for (k, v) in params {
                    q.append_pair(k, v);
                }
            }
            u.to_string()
        }
        Err(_) => "/".into(),
    }
}

/// Form or query parameters; `Err` when one appears twice (RFC 6749 section 3.1).
fn parse_params(s: &[u8]) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    for (k, v) in openidconnect::url::form_urlencoded::parse(s) {
        if m.insert(k.to_string(), v.to_string()).is_some() {
            return Err(format!("parameter {k} is repeated"));
        }
    }
    Ok(m)
}

// ---------------------------------------------------------------- in-memory state

/// A client as the consent page shows it.
#[derive(Debug, Clone)]
pub struct Client {
    pub id: String,
    pub name: String,
    /// `cimd` or `dcr`.
    pub kind: &'static str,
    pub redirect_uris: Vec<String>,
}

impl Client {
    /// The host of the client id URL (CIMD): the verified identity of the app.
    pub fn verified_host(&self) -> Option<String> {
        (self.kind == "cimd")
            .then(|| Url::parse(&self.id).ok()?.host_str().map(String::from))
            .flatten()
    }
}

struct Pending {
    client: Client,
    redirect_uri: String,
    scopes: Vec<String>,
    resource: String,
    challenge: String,
    state: Option<String>,
    csrf: String,
    addr: String,
    created: Instant,
}

#[derive(Clone)]
struct Code {
    client: Client,
    redirect_uri: String,
    scopes: Vec<String>,
    resource: String,
    challenge: String,
    user_id: i64,
    created: Instant,
    /// Set at the first use; then the grant made from it (a second use revokes it), or 0
    /// while it is being made.
    used_by: Option<i64>,
}

/// OAuth state kept in memory.
pub struct OAuth {
    pending: Mutex<HashMap<String, Pending>>,
    codes: Mutex<HashMap<String, Code>>,
    windows: Mutex<HashMap<String, (Instant, u32)>>,
    pub cimd: cimd::Cache,
}

impl OAuth {
    pub fn new(cfg: &crate::config::Config) -> OAuth {
        OAuth {
            pending: Mutex::new(HashMap::new()),
            codes: Mutex::new(HashMap::new()),
            windows: Mutex::new(HashMap::new()),
            cimd: cimd::Cache::new(&cfg.oauth_client_docs),
        }
    }

    /// Counts a request for `key`; `false` above `max` in the current `window`.
    fn allow(&self, key: String, max: u32, window: Duration) -> bool {
        let mut g = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        if g.len() > 10_000 {
            g.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(3600));
        }
        let w = g.entry(key).or_insert((now, 0));
        if now.duration_since(w.0) >= window {
            *w = (now, 0);
        }
        if w.1 >= max {
            return false;
        }
        w.1 += 1;
        true
    }

    /// Drops expired pending requests and codes.
    pub fn prune(&self) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, p| p.created.elapsed() < PENDING_TTL);
        self.codes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, c| c.created.elapsed() < CODE_TTL);
    }
}

// ---------------------------------------------------------------- router

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(authorization_server),
        )
        .route("/oauth/authorize", get(authorize))
        .route("/oauth/token", post(token))
        .route("/oauth/register", post(register))
        .route("/oauth/revoke", post(revoke))
}

fn no_store(mut r: Response) -> Response {
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r.headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    r
}

/// An OAuth error response (`{error, error_description}`).
fn oauth_error(status: StatusCode, code: &str, desc: &str) -> Response {
    no_store(
        (
            status,
            Json(json!({"error": code, "error_description": desc})),
        )
            .into_response(),
    )
}

fn disabled() -> Response {
    oauth_error(
        StatusCode::NOT_FOUND,
        "not_found",
        "OAuth is off: set FREELIB_PUBLIC_URL to the https:// address of this server",
    )
}

fn addr_key(st: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>) -> String {
    let ip = crate::auth::client_ip(headers, peer, st.cfg.trust_proxy);
    format!("{:?}", LimitKey::ip(ip))
}

fn metadata_json(v: Value) -> Response {
    let mut r = Json(v).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    r.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    r
}

/// `GET /.well-known/oauth-protected-resource[/mcp]` (RFC 9728).
pub async fn protected_resource(State(st): State<AppState>) -> Response {
    let Some(iss) = issuer(&st) else {
        return disabled();
    };
    metadata_json(json!({
        "resource": resource(&iss),
        "authorization_servers": [iss],
        "scopes_supported": SCOPES,
        "bearer_methods_supported": ["header"],
        "resource_name": "freeLib",
    }))
}

/// `GET /.well-known/oauth-authorization-server` (RFC 8414).
pub async fn authorization_server(State(st): State<AppState>) -> Response {
    let Some(iss) = issuer(&st) else {
        return disabled();
    };
    metadata_json(json!({
        "issuer": iss,
        "authorization_endpoint": format!("{iss}/oauth/authorize"),
        "token_endpoint": format!("{iss}/oauth/token"),
        "registration_endpoint": format!("{iss}/oauth/register"),
        "revocation_endpoint": format!("{iss}/oauth/revoke"),
        "scopes_supported": SCOPES,
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
    }))
}

// ---------------------------------------------------------------- clients

/// Finds the client of `client_id`: an `https://` URL is a Client ID Metadata Document on a
/// trusted host, anything else a registered client.
async fn resolve_client(st: &AppState, client_id: &str) -> Result<Client, String> {
    if client_id.is_empty() || client_id.len() > 512 {
        return Err("missing or invalid client_id".into());
    }
    if client_id.starts_with("https://") {
        let u = Url::parse(client_id).map_err(|_| "client_id is not a URL".to_string())?;
        if u.fragment().is_some()
            || !u.username().is_empty()
            || u.password().is_some()
            || matches!(u.path(), "" | "/")
        {
            return Err("client_id URL must have a path and no fragment or user info".into());
        }
        let host = u.host_str().unwrap_or("");
        if !host_trusted(host, &st.cfg.oauth_client_hosts) {
            return Err(format!(
                "client {host} is not trusted (add it to FREELIB_OAUTH_CLIENT_HOSTS)"
            ));
        }
        let d = st.oauth.cimd.get(client_id).await?;
        let uris: Vec<String> = d
            .redirect_uris
            .iter()
            .filter(|r| check_redirect_uri(r, &st.cfg.oauth_client_hosts).is_ok())
            .cloned()
            .collect();
        let name = d
            .client_name
            .map(|n| clean_name(&n))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| host.to_string());
        return Ok(Client {
            id: client_id.to_string(),
            name,
            kind: "cimd",
            redirect_uris: uris,
        });
    }
    let id = client_id.to_string();
    let c = st
        .db
        .run(move |c| store::get_client(c, &id))
        .await
        .map_err(|e| e.message)?
        .ok_or_else(|| "unknown client_id (register it first)".to_string())?;
    Ok(Client {
        id: c.client_id,
        name: c.name,
        kind: "dcr",
        redirect_uris: c.redirect_uris,
    })
}

fn clean_name(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect::<String>()
        .trim()
        .to_string()
}

#[derive(Deserialize)]
pub struct RegisterIn {
    #[serde(default)]
    redirect_uris: Vec<String>,
    client_name: Option<String>,
    client_uri: Option<String>,
    grant_types: Option<Vec<String>>,
    response_types: Option<Vec<String>>,
    token_endpoint_auth_method: Option<String>,
}

/// `POST /oauth/register`: Dynamic Client Registration (RFC 7591), public clients only.
pub async fn register(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if issuer(&st).is_none() {
        return disabled();
    }
    let key = addr_key(&st, &headers, peer.map(|p| p.0.0));
    if !st
        .oauth
        .allow(format!("reg:{key}"), 20, Duration::from_secs(3600))
    {
        return oauth_error(
            StatusCode::TOO_MANY_REQUESTS,
            "slow_down",
            "too many client registrations from this address",
        );
    }
    let bad = |m: &str| oauth_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", m);
    let b: RegisterIn = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(e) => return bad(&format!("invalid JSON: {e}")),
    };
    if b.redirect_uris.is_empty() || b.redirect_uris.len() > 10 {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "redirect_uris must list 1 to 10 URIs",
        );
    }
    for u in &b.redirect_uris {
        if let Err(e) = check_redirect_uri(u, &st.cfg.oauth_client_hosts) {
            return oauth_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri", &e);
        }
    }
    if let Some(g) = &b.grant_types
        && g.iter()
            .any(|x| x != "authorization_code" && x != "refresh_token")
    {
        return bad("only the authorization_code and refresh_token grant types are supported");
    }
    if let Some(r) = &b.response_types
        && r.iter().any(|x| x != "code")
    {
        return bad("only the code response type is supported");
    }
    if let Some(m) = &b.token_endpoint_auth_method
        && m != "none"
    {
        tracing::debug!("client asked for token_endpoint_auth_method {m}; registered as public");
    }
    let name = b
        .client_name
        .as_deref()
        .map(clean_name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "MCP client".into());
    let client_uri = b
        .client_uri
        .filter(|u| u.len() <= 512 && Url::parse(u).is_ok_and(|x| x.scheme() == "https"));
    let now = unix_now();
    let client = store::DcrClient {
        client_id: format!("flc_{}", random_token(18)),
        name,
        redirect_uris: b.redirect_uris,
        client_uri,
        created_at: now,
        last_used_at: None,
    };
    let c2 = client.clone();
    let res = st
        .db
        .run(move |c| {
            if store::count_clients(c)? >= MAX_CLIENTS {
                // make room: clients never used within the last hour
                store::delete_stale_clients(c, now, 3600, IDLE_CLIENT_SECS)?;
                if store::count_clients(c)? >= MAX_CLIENTS {
                    return Ok(false);
                }
            }
            store::insert_client(c, &c2)?;
            Ok(true)
        })
        .await;
    match res {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!("OAuth client registration refused: {MAX_CLIENTS} clients registered");
            return oauth_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily_unavailable",
                "too many registered clients; try again later",
            );
        }
        Err(e) => return e.into_response(),
    }
    tracing::info!(client = %client.name, id = %client.client_id, "OAuth client registered");
    no_store(
        (
            StatusCode::CREATED,
            Json(json!({
                "client_id": client.client_id,
                "client_id_issued_at": now,
                "client_name": client.name,
                "client_uri": client.client_uri,
                "redirect_uris": client.redirect_uris,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            })),
        )
            .into_response(),
    )
}

// ---------------------------------------------------------------- authorize

fn see_other(location: &str) -> Response {
    let mut r = StatusCode::SEE_OTHER.into_response();
    if let Ok(v) = HeaderValue::from_str(location) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    no_store(r)
}

/// An error shown by the web app (the client or its redirect URI cannot be trusted, so the
/// browser is never sent back to it).
fn error_page(code: &str) -> Response {
    see_other(&format!("/oauth/consent?error={code}"))
}

/// `GET /oauth/authorize`: validates the request and sends the browser to the consent page.
pub async fn authorize(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    RawQuery(q): RawQuery,
) -> Response {
    let Some(iss) = issuer(&st) else {
        return error_page("disabled");
    };
    let addr = addr_key(&st, &headers, peer.map(|p| p.0.0));
    if !st
        .oauth
        .allow(format!("authz:{addr}"), 60, Duration::from_secs(60))
    {
        return error_page("rate_limited");
    }
    let Ok(p) = parse_params(q.as_deref().unwrap_or("").as_bytes()) else {
        return error_page("invalid_request");
    };
    let get = |k: &str| p.get(k).map(String::as_str);
    let client = match resolve_client(&st, get("client_id").unwrap_or("")).await {
        Ok(c) => c,
        Err(e) => {
            tracing::info!("OAuth authorization refused: {e}");
            return error_page("invalid_client");
        }
    };
    let Some(redirect_uri) = get("redirect_uri").map(str::to_string) else {
        return error_page("invalid_redirect_uri");
    };
    if check_redirect_uri(&redirect_uri, &st.cfg.oauth_client_hosts).is_err()
        || !client
            .redirect_uris
            .iter()
            .any(|r| redirect_matches(r, &redirect_uri))
    {
        tracing::info!(client = %client.name, "OAuth authorization refused: redirect URI {redirect_uri} not registered");
        return error_page("invalid_redirect_uri");
    }
    // from here on errors go back to the client
    let state = get("state").map(str::to_string);
    let back = |code: &str, desc: &str| {
        let mut ps = vec![("error", code), ("error_description", desc)];
        if let Some(s) = &state {
            ps.push(("state", s));
        }
        ps.push(("iss", &iss));
        see_other(&with_params(&redirect_uri, &ps))
    };
    if get("response_type") != Some("code") {
        return back(
            "unsupported_response_type",
            "only response_type=code is supported",
        );
    }
    if state.as_ref().is_some_and(|s| s.len() > 2048) {
        return back("invalid_request", "state too long");
    }
    let challenge = get("code_challenge").unwrap_or("");
    if get("code_challenge_method") != Some("S256")
        || challenge.len() != 43
        || !pkce_chars_ok(challenge)
    {
        return back(
            "invalid_request",
            "PKCE is required: code_challenge with code_challenge_method=S256",
        );
    }
    let scopes = match parse_scopes(get("scope")) {
        Ok(s) => s,
        Err(e) => return back("invalid_scope", &e),
    };
    let Some(res) = check_resource(&iss, get("resource")) else {
        return back(
            "invalid_target",
            &format!("this server only issues tokens for {}", resource(&iss)),
        );
    };
    let id = random_token(24);
    {
        let mut g = st.oauth.pending.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|_, p| p.created.elapsed() < PENDING_TTL);
        if g.len() >= MAX_PENDING
            || g.values().filter(|p| p.addr == addr).count() >= MAX_PENDING_PER_ADDR
        {
            drop(g);
            return error_page("rate_limited");
        }
        g.insert(
            id.clone(),
            Pending {
                client,
                redirect_uri,
                scopes,
                resource: res,
                challenge: challenge.to_string(),
                state,
                csrf: random_token(24),
                addr,
                created: Instant::now(),
            },
        );
    }
    see_other(&format!("/oauth/consent?request={id}"))
}

/// What the consent page shows (`GET /api/v1/oauth/requests/{id}`).
pub fn pending_view(st: &AppState, id: &str) -> Option<Value> {
    let g = st.oauth.pending.lock().unwrap_or_else(|e| e.into_inner());
    let p = g.get(id).filter(|p| p.created.elapsed() < PENDING_TTL)?;
    let ru = Url::parse(&p.redirect_uri).ok()?;
    let host = ru.host_str().unwrap_or("").to_string();
    let loopback = is_loopback_host(&host);
    Some(json!({
        "client": {
            "name": p.client.name,
            "kind": p.client.kind,
            "verifiedHost": p.client.verified_host(),
            "clientUri": if p.client.kind == "cimd" { Some(p.client.id.clone()) } else { None },
        },
        "redirectUri": p.redirect_uri,
        "redirectHost": host,
        "loopback": loopback,
        "scopes": p.scopes,
        "resource": p.resource,
        "csrf": p.csrf,
    }))
}

/// The user's answer on the consent page. Returns where to send the browser.
pub enum Decision {
    Approve(Vec<String>),
    Deny,
}

/// Why a decision cannot be applied.
pub enum DecisionError {
    NotFound,
    Csrf,
    Scopes(String),
}

/// Applies the consent decision of `user` for pending request `id`; returns the redirect URL
/// (with `code` or `error=access_denied`, `state` and `iss`).
pub async fn decide(
    st: &AppState,
    user: &User,
    id: &str,
    csrf: &str,
    d: Decision,
) -> Result<String, DecisionError> {
    let iss = issuer(st).ok_or(DecisionError::NotFound)?;
    let p = {
        let mut g = st.oauth.pending.lock().unwrap_or_else(|e| e.into_inner());
        let p = g
            .get(id)
            .filter(|p| p.created.elapsed() < PENDING_TTL)
            .ok_or(DecisionError::NotFound)?;
        if !bool::from(subtle::ConstantTimeEq::ct_eq(
            p.csrf.as_bytes(),
            csrf.as_bytes(),
        )) {
            return Err(DecisionError::Csrf);
        }
        if let Decision::Approve(s) = &d {
            if s.is_empty() {
                return Err(DecisionError::Scopes(
                    "choose at least one permission".into(),
                ));
            }
            if let Some(x) = s.iter().find(|x| !p.scopes.contains(x)) {
                return Err(DecisionError::Scopes(format!(
                    "scope {x} was not requested by the app"
                )));
            }
        }
        g.remove(id).ok_or(DecisionError::NotFound)?
    };
    let mut ps: Vec<(&str, String)> = Vec::new();
    let (ok, detail) = match d {
        Decision::Deny => {
            ps.push(("error", "access_denied".into()));
            ps.push(("error_description", "the user denied access".into()));
            (false, format!("{} denied", p.client.name))
        }
        Decision::Approve(scopes) => {
            let scopes: Vec<String> = SCOPES
                .iter()
                .filter(|x| scopes.iter().any(|s| s == *x))
                .map(|s| s.to_string())
                .collect();
            let code = random_token(32);
            let detail = format!(
                "{} → {} ({})",
                p.client.name,
                Url::parse(&p.redirect_uri)
                    .ok()
                    .and_then(|u| u.host_str().map(String::from))
                    .unwrap_or_default(),
                scopes.join(" ")
            );
            {
                let mut g = st.oauth.codes.lock().unwrap_or_else(|e| e.into_inner());
                g.retain(|_, c| c.created.elapsed() < CODE_TTL);
                if g.len() >= MAX_CODES {
                    g.clear();
                }
                g.insert(
                    sha256_hex(code.as_bytes()),
                    Code {
                        client: p.client.clone(),
                        redirect_uri: p.redirect_uri.clone(),
                        scopes,
                        resource: p.resource.clone(),
                        challenge: p.challenge.clone(),
                        user_id: user.id,
                        created: Instant::now(),
                        used_by: None,
                    },
                );
            }
            ps.push(("code", code));
            (true, detail)
        }
    };
    if let Some(s) = &p.state {
        ps.push(("state", s.clone()));
    }
    ps.push(("iss", iss));
    let uid = user.id;
    let cid = p.client.id.clone();
    let dcr = p.client.kind == "dcr";
    let _ = st
        .db
        .run(move |c| {
            if ok && dcr {
                store::touch_client(c, &cid, unix_now())?;
            }
            db::add_audit(c, uid, None, None, "oauth.authorize", ok, &detail)
        })
        .await;
    tracing::info!(user = %user.username, "OAuth consent: {}", if ok { "approved" } else { "denied" });
    let pairs: Vec<(&str, &str)> = ps.iter().map(|(k, v)| (*k, v.as_str())).collect();
    Ok(with_params(&p.redirect_uri, &pairs))
}

// ---------------------------------------------------------------- token

fn token_response(access: &str, refresh: &str, scopes: &[String]) -> Response {
    no_store(
        Json(json!({
            "access_token": access,
            "token_type": "Bearer",
            "expires_in": ACCESS_TTL,
            "refresh_token": refresh,
            "scope": scopes.join(" "),
        }))
        .into_response(),
    )
}

fn invalid_grant(desc: &str) -> Response {
    oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", desc)
}

/// The client id of a token or revocation request: the `client_id` parameter, or the user
/// name of HTTP Basic authentication (the password is ignored: no client has a secret).
fn request_client_id(p: &HashMap<String, String>, headers: &HeaderMap) -> Option<String> {
    if let Some(c) = p.get("client_id") {
        return Some(c.clone());
    }
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, rest) = v.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(rest.trim())
        .ok()?;
    let s = String::from_utf8(raw).ok()?;
    let user = s.split(':').next()?;
    let dec: String = openidconnect::url::form_urlencoded::parse(format!("a={user}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())?;
    (!dec.is_empty()).then_some(dec)
}

fn form_params(headers: &HeaderMap, body: &[u8]) -> Result<HashMap<String, String>, Response> {
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ct
        .to_ascii_lowercase()
        .starts_with("application/x-www-form-urlencoded")
    {
        return Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "the body must be application/x-www-form-urlencoded",
        ));
    }
    parse_params(body).map_err(|e| oauth_error(StatusCode::BAD_REQUEST, "invalid_request", &e))
}

/// `POST /oauth/token`: `authorization_code` (with PKCE) and `refresh_token` (rotating).
pub async fn token(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(iss) = issuer(&st) else {
        return disabled();
    };
    let addr = addr_key(&st, &headers, peer.map(|p| p.0.0));
    if !st
        .oauth
        .allow(format!("token:{addr}"), 120, Duration::from_secs(60))
    {
        return oauth_error(
            StatusCode::TOO_MANY_REQUESTS,
            "slow_down",
            "too many token requests from this address",
        );
    }
    let p = match form_params(&headers, &body) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(client_id) = request_client_id(&p, &headers) else {
        return oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client_id is required",
        );
    };
    let r = match p.get("grant_type").map(String::as_str) {
        Some("authorization_code") => code_grant(&st, &iss, &p, &client_id).await,
        Some("refresh_token") => refresh_grant(&st, &iss, &p, &client_id).await,
        Some(_) => Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "use authorization_code or refresh_token",
        )),
        None => Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "grant_type is required",
        )),
    };
    r.unwrap_or_else(|e| e)
}

async fn code_grant(
    st: &AppState,
    iss: &str,
    p: &HashMap<String, String>,
    client_id: &str,
) -> Result<Response, Response> {
    let code = p.get("code").ok_or_else(|| {
        oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "code is required",
        )
    })?;
    let verifier = p.get("code_verifier").ok_or_else(|| {
        oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "code_verifier is required (PKCE)",
        )
    })?;
    let key = sha256_hex(code.as_bytes());
    // take the code (single use); a second use revokes what the first one produced
    type Taken = Result<Code, (Response, Option<(i64, i64)>)>;
    let taken: Taken = (|| {
        let mut g = st.oauth.codes.lock().unwrap_or_else(|e| e.into_inner());
        let Some(c) = g.get_mut(&key).filter(|c| c.created.elapsed() < CODE_TTL) else {
            return Err((invalid_grant("unknown, expired or already used code"), None));
        };
        if let Some(grant) = c.used_by {
            let reuse = (grant > 0).then_some((grant, c.user_id));
            return Err((
                invalid_grant("unknown, expired or already used code"),
                reuse,
            ));
        }
        if c.client.id != client_id {
            return Err((invalid_grant("the code was issued to another client"), None));
        }
        if p.get("redirect_uri") != Some(&c.redirect_uri) {
            return Err((
                invalid_grant("redirect_uri does not match the authorization request"),
                None,
            ));
        }
        if !pkce_ok(verifier, &c.challenge) {
            return Err((invalid_grant("PKCE verification failed"), None));
        }
        if let Some(r) = p.get("resource").filter(|r| !r.is_empty())
            && check_resource(iss, Some(r)).as_deref() != Some(c.resource.as_str())
        {
            return Err((
                oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_target",
                    "resource does not match the authorization request",
                ),
                None,
            ));
        }
        c.used_by = Some(0);
        Ok(c.clone())
    })();
    let c = match taken {
        Ok(c) => c,
        Err((resp, reuse)) => {
            if let Some((grant, uid)) = reuse {
                tracing::warn!("OAuth authorization code used twice: grant {grant} revoked");
                let _ = st
                    .db
                    .run(move |c| {
                        store::revoke_grant(c, grant)?;
                        db::add_audit(
                            c,
                            uid,
                            None,
                            Some(grant),
                            "oauth.code_reuse",
                            false,
                            "authorization code used twice: access revoked",
                        )
                    })
                    .await;
                st.tokens.invalidate();
            }
            return Err(resp);
        }
    };
    let access = new_secret(ACCESS_PREFIX);
    let refresh = new_secret(REFRESH_PREFIX);
    let now = unix_now();
    let (ah, rh) = (
        sha256_hex(access.as_bytes()),
        sha256_hex(refresh.as_bytes()),
    );
    let c2 = c;
    let grant = st
        .db
        .run(move |db| {
            store::create_grant(
                db,
                &store::NewGrant {
                    user_id: c2.user_id,
                    client_id: &c2.client.id,
                    client_name: &c2.client.name,
                    client_kind: c2.client.kind,
                    redirect_uri: &c2.redirect_uri,
                    scopes: &c2.scopes,
                    resource: &c2.resource,
                },
                (&ah, now + ACCESS_TTL),
                (&rh, now + REFRESH_TTL),
            )
            .map(|g| (g, c2.scopes))
        })
        .await;
    let (grant, scopes) = match grant {
        Ok(x) => x,
        Err(e) => return Err(e.into_response()),
    };
    if let Some(c) = st
        .oauth
        .codes
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(&key)
    {
        c.used_by = Some(grant);
    }
    Ok(token_response(&access, &refresh, &scopes))
}

async fn refresh_grant(
    st: &AppState,
    iss: &str,
    p: &HashMap<String, String>,
    client_id: &str,
) -> Result<Response, Response> {
    let rt = p.get("refresh_token").ok_or_else(|| {
        oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "refresh_token is required",
        )
    })?;
    if !well_formed(rt, REFRESH_PREFIX) {
        return Err(invalid_grant("invalid refresh token"));
    }
    let hash = sha256_hex(rt.as_bytes());
    let h2 = hash.clone();
    let row = st
        .db
        .run(move |c| store::refresh_token(c, &h2))
        .await
        .map_err(IntoResponse::into_response)?
        .ok_or_else(|| invalid_grant("invalid, expired or revoked refresh token"))?;
    let now = unix_now();
    if row.client_id != client_id {
        return Err(invalid_grant(
            "the refresh token was issued to another client",
        ));
    }
    if row.rotated_at.is_some() {
        // reuse of a rotated token: someone else has a copy; end the grant for everyone
        let (g, uid, name) = (row.grant_id, row.user_id, row.client_name.clone());
        tracing::warn!(client = %name, "OAuth refresh token reused: grant {g} revoked");
        let _ = st
            .db
            .run(move |c| {
                store::revoke_grant(c, g)?;
                db::add_audit(
                    c,
                    uid,
                    None,
                    Some(g),
                    "oauth.refresh_reuse",
                    false,
                    &format!("{name}: a used refresh token was presented again; access revoked"),
                )
            })
            .await;
        st.tokens.invalidate();
        return Err(invalid_grant("refresh token already used; access revoked"));
    }
    if row.expires_at <= now {
        return Err(invalid_grant("refresh token expired"));
    }
    if let Some(r) = p.get("resource").filter(|r| !r.is_empty())
        && check_resource(iss, Some(r)).as_deref() != Some(row.resource.as_str())
    {
        return Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "resource does not match the grant",
        ));
    }
    let scopes = match p.get("scope") {
        Some(s) => {
            let asked = parse_scopes(Some(s))
                .map_err(|e| oauth_error(StatusCode::BAD_REQUEST, "invalid_scope", &e))?;
            if let Some(x) = asked.iter().find(|x| !row.scopes.contains(x)) {
                return Err(oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_scope",
                    &format!("scope {x} was not granted"),
                ));
            }
            asked
        }
        None => row.scopes.clone(),
    };
    let access = new_secret(ACCESS_PREFIX);
    let refresh = new_secret(REFRESH_PREFIX);
    let (ah, rh) = (
        sha256_hex(access.as_bytes()),
        sha256_hex(refresh.as_bytes()),
    );
    let (g, sc) = (row.grant_id, scopes.join(" "));
    let ok = st
        .db
        .run(move |c| {
            store::rotate(
                c,
                &hash,
                g,
                now,
                (&ah, now + ACCESS_TTL, &sc),
                (&rh, now + REFRESH_TTL),
            )
        })
        .await
        .map_err(IntoResponse::into_response)?;
    if !ok {
        return Err(invalid_grant("refresh token already used"));
    }
    Ok(token_response(&access, &refresh, &scopes))
}

// ---------------------------------------------------------------- revoke

/// `POST /oauth/revoke` (RFC 7009): always 200 for a well-formed request.
pub async fn revoke(
    State(st): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if issuer(&st).is_none() {
        return disabled();
    }
    let addr = addr_key(&st, &headers, peer.map(|p| p.0.0));
    if !st
        .oauth
        .allow(format!("token:{addr}"), 120, Duration::from_secs(60))
    {
        return oauth_error(
            StatusCode::TOO_MANY_REQUESTS,
            "slow_down",
            "too many requests from this address",
        );
    }
    let p = match form_params(&headers, &body) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(tok) = p.get("token").filter(|t| !t.is_empty()) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "token is required",
        );
    };
    if well_formed(tok, ACCESS_PREFIX) || well_formed(tok, REFRESH_PREFIX) {
        let hash = sha256_hex(tok.as_bytes());
        let cid = request_client_id(&p, &headers);
        let res = st
            .db
            .run(move |c| {
                let r = store::revoke_token(c, &hash, cid.as_deref())?;
                if let Some((g, uid, name)) = &r {
                    db::add_audit(
                        c,
                        *uid,
                        None,
                        Some(*g),
                        "oauth.revoke",
                        true,
                        &format!("{name}: token revoked by the app"),
                    )?;
                }
                Ok(r)
            })
            .await;
        if let Err(e) = res {
            return e.into_response();
        }
        st.tokens.invalidate();
    }
    no_store(StatusCode::OK.into_response())
}

// ---------------------------------------------------------------- resource server side

/// A valid OAuth access token for this MCP endpoint.
pub struct AccessAuth {
    pub row: store::AccessRow,
    pub user: User,
}

/// Looks up an access token: unexpired, of an existing grant and user, bound to this server's
/// MCP resource (audience).
pub async fn authenticate(st: &AppState, secret: &str) -> ApiResult<Option<AccessAuth>> {
    let Some(iss) = issuer(st) else {
        return Ok(None);
    };
    if !is_access_token(secret) {
        return Ok(None);
    }
    let h = sha256_hex(secret.as_bytes());
    let now = unix_now();
    let found = st
        .db
        .run(move |c| {
            let r = store::access_token(c, &h, now)?;
            if let Some((a, _)) = &r {
                store::touch_grant(c, a.grant_id, now)?;
            }
            Ok(r)
        })
        .await?;
    let Some((row, user)) = found else {
        return Ok(None);
    };
    if row.resource != resource(&iss) {
        return Ok(None);
    }
    Ok(Some(AccessAuth { row, user }))
}

/// Periodic cleanup: expired tokens, empty grants, stale registered clients, in-memory state.
pub async fn cleanup(st: &AppState) {
    st.oauth.prune();
    let now = unix_now();
    let r = st
        .db
        .run(move |c| {
            let t = store::cleanup(c, now)?;
            let cl = store::delete_stale_clients(c, now, UNUSED_CLIENT_SECS, IDLE_CLIENT_SECS)?;
            Ok((t, cl))
        })
        .await;
    match r {
        Ok(((t, g), cl)) if t + g + cl > 0 => tracing::debug!(
            "OAuth cleanup: {t} expired tokens, {g} grants, {cl} unused clients removed"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!("OAuth cleanup failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trusted() -> Vec<String> {
        crate::config::default_client_hosts()
    }

    #[test]
    fn redirect_policy() {
        let t = trusted();
        for ok in [
            "https://claude.ai/api/mcp/auth_callback",
            "https://claude.com/api/mcp/auth_callback",
            "http://127.0.0.1:33418/callback",
            "http://localhost/callback",
            "http://[::1]:8000/cb?x=1",
        ] {
            assert!(check_redirect_uri(ok, &t).is_ok(), "{ok}");
        }
        for bad in [
            "https://evil.example/cb",
            "https://claude.ai.evil.example/cb",
            "https://evilclaude.ai/cb",
            "http://claude.ai/api/mcp/auth_callback",
            "http://192.168.1.2/cb",
            "http://localhost.evil.example/cb",
            "https://claude.ai/cb#frag",
            "https://user:pw@claude.ai/cb",
            "https://claude.ai@evil.example/cb",
            "javascript:alert(1)",
            "data:text/html,x",
            "cursor://callback",
            "/relative",
            "https://1.2.3.4/cb",
            "https://claude.ai/cb\r\nX: y",
        ] {
            assert!(check_redirect_uri(bad, &t).is_err(), "{bad}");
        }
        assert!(check_redirect_uri("https://app.example/cb", &["*".into()]).is_ok());
        assert!(
            check_redirect_uri("https://a.corp.example/cb", &["*.corp.example".into()]).is_ok()
        );
        assert!(check_redirect_uri("https://corp.example/cb", &["*.corp.example".into()]).is_err());
    }

    #[test]
    fn redirect_matching() {
        assert!(redirect_matches(
            "https://claude.ai/api/mcp/auth_callback",
            "https://claude.ai/api/mcp/auth_callback"
        ));
        assert!(!redirect_matches(
            "https://claude.ai/api/mcp/auth_callback",
            "https://claude.ai/api/mcp/auth_callback/"
        ));
        assert!(!redirect_matches(
            "https://claude.ai/api/mcp/auth_callback",
            "https://claude.ai/api/mcp/auth_callback?x=1"
        ));
        assert!(!redirect_matches(
            "https://claude.ai:443/a",
            "https://claude.ai:8443/a"
        ));
        // loopback: any port
        assert!(redirect_matches(
            "http://localhost/callback",
            "http://localhost:3118/callback"
        ));
        assert!(redirect_matches(
            "http://127.0.0.1:1/callback",
            "http://127.0.0.1:55555/callback"
        ));
        assert!(!redirect_matches(
            "http://localhost/callback",
            "http://127.0.0.1:3118/callback"
        ));
        assert!(!redirect_matches(
            "http://localhost/callback",
            "http://localhost:3118/other"
        ));
        assert!(!redirect_matches(
            "http://localhost/callback",
            "https://localhost:3118/callback"
        ));
        assert!(!redirect_matches(
            "http://localhost/callback",
            "http://localhost:3118/callback#x"
        ));
    }

    #[test]
    fn scopes_and_resource() {
        assert_eq!(parse_scopes(None).unwrap(), ["read", "write", "send"]);
        assert_eq!(
            parse_scopes(Some("send read offline_access")).unwrap(),
            ["read", "send"]
        );
        assert_eq!(
            parse_scopes(Some("offline_access")).unwrap(),
            ["read", "write", "send"]
        );
        assert!(parse_scopes(Some("read admin")).is_err());
        let iss = "https://books.example.org";
        let canon = "https://books.example.org/mcp";
        for ok in [
            None,
            Some(""),
            Some(canon),
            Some("HTTPS://Books.Example.org/mcp"),
            Some("https://books.example.org/mcp/"),
            Some(iss),
            Some("https://books.example.org/"),
        ] {
            assert_eq!(check_resource(iss, ok).unwrap(), canon, "{ok:?}");
        }
        for bad in [
            "https://other.example/mcp",
            "https://books.example.org/api",
            "https://books.example.org/mcp#x",
            "books.example.org/mcp",
            "https://books.example.org.evil/mcp",
            "http://books.example.org/mcp",
        ] {
            assert!(check_resource(iss, Some(bad)).is_none(), "{bad}");
        }
    }

    #[test]
    fn pkce() {
        // RFC 7636 appendix B
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let c = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(pkce_ok(v, c));
        assert!(!pkce_ok("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXX", c));
        assert!(!pkce_ok("short", c));
    }

    #[test]
    fn params() {
        assert!(parse_params(b"a=1&b=2").is_ok());
        assert!(parse_params(b"a=1&a=2").is_err());
        assert_eq!(
            with_params(
                "https://claude.ai/cb?x=1",
                &[("code", "a b"), ("state", "s&t")]
            ),
            "https://claude.ai/cb?x=1&code=a+b&state=s%26t"
        );
        assert!(is_access_token(&new_secret(ACCESS_PREFIX)));
        assert!(!is_access_token(&new_secret(REFRESH_PREFIX)));
    }
}
