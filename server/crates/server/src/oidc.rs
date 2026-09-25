//! OpenID Connect sign-in (`FREELIB_OIDC_*`): discovery, the authorization code flow with
//! PKCE (S256), `state` and `nonce`, ID token validation and account linking. The HTTP
//! handlers are in [`crate::api::oidc`]; see docs/web/ARCHITECTURE.md "Single sign-on".
//!
//! * Provider metadata and its JWKS are cached for an hour; an ID token signed with an unknown
//!   key triggers one early refresh (key rotation). That needs a token the provider just
//!   issued for this client, so it cannot be used to make the server hammer the provider.
//! * Pending sign-ins (state → nonce, PKCE verifier, return path, user being linked) live in
//!   memory for 10 minutes and are single use. The browser holds the state in an HttpOnly
//!   cookie scoped to the callback, which must match the `state` the provider sends back.
//! * ID tokens: signature (asymmetric algorithms only), `iss`, `aud`, `exp` (60 s leeway),
//!   `iat` (not in the future, not older than 10 minutes), `nonce`, and `at_hash` when present.
//! * Accounts are found by (issuer, subject) only, never by user name or e-mail.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreClientAuthMethod, CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm, CoreProviderMetadata,
};
use openidconnect::{
    AccessTokenHash, AdditionalClaims, AuthType, AuthorizationCode, ClaimsVerificationError,
    ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet, EndpointSet, IdToken,
    IdTokenClaims, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, Scope, SignatureVerificationError, TokenResponse,
};
use serde::{Deserialize, Serialize};

use crate::config::OidcConfig;
use crate::db::{self, User};
use crate::state::AppState;

/// Path of the redirect URI under `FREELIB_PUBLIC_URL`.
pub const CALLBACK_PATH: &str = "/api/v1/auth/oidc/callback";
/// Cookie holding the `state` of a sign-in in progress.
pub const STATE_COOKIE: &str = "freelib_oidc";
/// Path the state cookie is scoped to.
pub const STATE_COOKIE_PATH: &str = "/api/v1/auth/oidc";
/// Lifetime of a pending sign-in.
pub const PENDING_TTL: Duration = Duration::from_secs(600);
const MAX_PENDING: usize = 10_000;
const MAX_PER_CLIENT: usize = 20;
const META_TTL: Duration = Duration::from_secs(3600);
const LEEWAY_SECS: i64 = 60;
const MAX_TOKEN_AGE_SECS: i64 = 600;

/// Claims beyond the standard ones: `groups` (a list of names, or one name).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtraClaims {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groups: Option<serde_json::Value>,
}
impl AdditionalClaims for ExtraClaims {}

type IdTok = IdToken<
    ExtraClaims,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;
type Claims = IdTokenClaims<ExtraClaims, CoreGenderClaim>;
type Client = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// Why a sign-in failed. The code goes to the web app (`/login?ssoError=<code>`); the detail
/// is only logged.
#[derive(Debug)]
pub struct SsoError {
    pub code: &'static str,
    pub detail: String,
}

impl SsoError {
    fn new(code: &'static str, detail: impl Into<String>) -> SsoError {
        SsoError {
            code,
            detail: detail.into(),
        }
    }
    /// The sign-in attempt is unknown, expired, already used, or not this browser's.
    pub fn state(detail: impl Into<String>) -> SsoError {
        Self::new("state", detail)
    }
    /// The provider cannot be reached or its discovery document is unusable.
    pub fn unavailable(detail: impl Into<String>) -> SsoError {
        Self::new("unavailable", detail)
    }
    /// Token exchange or ID token validation failed.
    pub fn token(detail: impl Into<String>) -> SsoError {
        Self::new("token", detail)
    }
}

impl std::fmt::Display for SsoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

struct Pending {
    nonce: Nonce,
    verifier: String,
    return_to: String,
    link_user: Option<i64>,
    /// Who started it (rate limiting key), so one client cannot fill the table.
    client: String,
    created: Instant,
}

/// What the provider told us about the user (after validation).
#[derive(Debug, Clone)]
pub struct Profile {
    pub issuer: String,
    pub subject: String,
    pub preferred_username: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub groups: Vec<String>,
}

/// A finished callback.
pub struct Verified {
    pub profile: Profile,
    pub return_to: String,
    pub link_user: Option<i64>,
}

pub struct Provider {
    pub cfg: OidcConfig,
    issuer: IssuerUrl,
    redirect: RedirectUrl,
    http: openidconnect::reqwest::Client,
    meta: tokio::sync::Mutex<Option<(CoreProviderMetadata, Instant)>>,
    pending: Mutex<HashMap<String, Pending>>,
}

impl Provider {
    /// Checks the configuration; no network access.
    pub fn new(cfg: OidcConfig, public_url: Option<&str>) -> Result<Provider, String> {
        let public = public_url.ok_or(
            "FREELIB_PUBLIC_URL must be set for single sign-on (the redirect URI is \
             <FREELIB_PUBLIC_URL>/api/v1/auth/oidc/callback)",
        )?;
        let parsed = openidconnect::url::Url::parse(public)
            .map_err(|e| format!("FREELIB_PUBLIC_URL is not a URL: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err("FREELIB_PUBLIC_URL must be an http(s) URL".into());
        }
        let redirect = RedirectUrl::new(format!("{}{CALLBACK_PATH}", public.trim_end_matches('/')))
            .map_err(|e| format!("bad redirect URI: {e}"))?;
        let issuer = IssuerUrl::new(cfg.issuer.clone())
            .map_err(|e| format!("FREELIB_OIDC_ISSUER is not a URL: {e}"))?;
        let http = openidconnect::reqwest::ClientBuilder::new()
            // following redirects would expose the server to SSRF through the provider
            .redirect(openidconnect::reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("freelib-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("HTTP client: {e}"))?;
        Ok(Provider {
            cfg,
            issuer,
            redirect,
            http,
            meta: tokio::sync::Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub fn issuer(&self) -> &str {
        self.issuer.as_str()
    }

    pub fn redirect_uri(&self) -> &str {
        self.redirect.as_str()
    }

    /// Provider metadata with its JWKS: cached for [`META_TTL`]; `force` refetches. A failed
    /// refresh keeps using the cached copy.
    async fn metadata(&self, force: bool) -> Result<CoreProviderMetadata, SsoError> {
        let mut g = self.meta.lock().await;
        if let Some((m, at)) = &*g
            && !force
            && at.elapsed() < META_TTL
        {
            return Ok(m.clone());
        }
        match CoreProviderMetadata::discover_async(self.issuer.clone(), &self.http).await {
            Ok(m) => {
                *g = Some((m.clone(), Instant::now()));
                Ok(m)
            }
            Err(e) => {
                let msg = format!(
                    "discovery of {} failed: {}",
                    self.issuer.as_str(),
                    chain(&e)
                );
                match &*g {
                    Some((m, _)) => {
                        tracing::warn!("{msg}; using the cached provider metadata");
                        Ok(m.clone())
                    }
                    None => Err(SsoError::unavailable(msg)),
                }
            }
        }
    }

    /// Fetches the discovery document once (startup check; errors are only logged).
    pub async fn probe(&self) {
        match self.metadata(false).await {
            Ok(_) => tracing::info!(
                issuer = self.issuer.as_str(),
                redirect_uri = self.redirect.as_str(),
                "single sign-on ready"
            ),
            Err(e) => tracing::warn!("single sign-on: {e} (retried at the next sign-in)"),
        }
    }

    fn client(&self, meta: CoreProviderMetadata) -> Client {
        // client_secret_basic unless the provider only lists client_secret_post
        let post_only = meta
            .token_endpoint_auth_methods_supported()
            .is_some_and(|m| {
                !m.contains(&CoreClientAuthMethod::ClientSecretBasic)
                    && m.contains(&CoreClientAuthMethod::ClientSecretPost)
            });
        let c = CoreClient::from_provider_metadata(
            meta,
            ClientId::new(self.cfg.client_id.clone()),
            self.cfg.client_secret.clone().map(ClientSecret::new),
        )
        .set_redirect_uri(self.redirect.clone());
        if post_only {
            c.set_auth_type(AuthType::RequestBody)
        } else {
            c
        }
    }

    /// Starts a sign-in: returns the provider's authorization URL and the `state` to put in
    /// the state cookie.
    pub async fn begin(
        &self,
        return_to: String,
        link_user: Option<i64>,
        client_key: String,
    ) -> Result<(String, String), SsoError> {
        let meta = self.metadata(false).await?;
        let client = self.client(meta);
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut req = client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        );
        for s in &self.cfg.scopes {
            if s != "openid" {
                req = req.add_scope(Scope::new(s.clone()));
            }
        }
        let (url, state, nonce) = req.set_pkce_challenge(challenge).url();
        let state = state.secret().clone();
        {
            let mut g = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            g.retain(|_, p| p.created.elapsed() < PENDING_TTL);
            // one client keeps at most MAX_PER_CLIENT sign-ins in progress: its oldest go first
            let mut mine: Vec<(Instant, String)> = g
                .iter()
                .filter(|(_, p)| p.client == client_key)
                .map(|(k, p)| (p.created, k.clone()))
                .collect();
            if mine.len() >= MAX_PER_CLIENT {
                mine.sort();
                for (_, k) in &mine[..=mine.len() - MAX_PER_CLIENT] {
                    g.remove(k);
                }
            }
            if g.len() >= MAX_PENDING {
                return Err(SsoError::new(
                    "busy",
                    "too many sign-ins in progress, retry in a few minutes",
                ));
            }
            g.insert(
                state.clone(),
                Pending {
                    nonce,
                    verifier: verifier.secret().clone(),
                    return_to,
                    link_user,
                    client: client_key,
                    created: Instant::now(),
                },
            );
        }
        Ok((url.to_string(), state))
    }

    /// Takes the pending sign-in of `state` (single use) after checking it against the cookie.
    fn take(&self, state: &str, cookie: Option<&str>) -> Result<Pending, SsoError> {
        let cookie = cookie.ok_or_else(|| SsoError::state("no state cookie"))?;
        if !bool::from(subtle::ConstantTimeEq::ct_eq(
            state.as_bytes(),
            cookie.as_bytes(),
        )) {
            return Err(SsoError::state("state does not match the cookie"));
        }
        let p = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(state)
            .ok_or_else(|| SsoError::state("unknown or already used state"))?;
        if p.created.elapsed() >= PENDING_TTL {
            return Err(SsoError::state("sign-in attempt expired"));
        }
        Ok(p)
    }

    /// The link flow of a failed callback, to send the user back to the right page.
    pub fn peek_link(&self, state: Option<&str>) -> bool {
        state.is_some_and(|s| {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(s)
                .is_some_and(|p| p.link_user.is_some())
        })
    }

    /// Finishes the callback: state, code exchange, ID token validation, userinfo.
    pub async fn finish(
        &self,
        q: &CallbackQuery,
        cookie: Option<&str>,
    ) -> Result<Verified, SsoError> {
        let state = q
            .state
            .as_deref()
            .ok_or_else(|| SsoError::state("no state parameter"))?;
        let pending = self.take(state, cookie)?;
        if let Some(err) = &q.error {
            let code: String = err
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(64)
                .collect();
            return Err(SsoError::new(
                "provider",
                format!(
                    "{code}: {}",
                    q.error_description
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(300)
                        .collect::<String>()
                ),
            ));
        }
        let code = q
            .code
            .clone()
            .ok_or_else(|| SsoError::token("no authorization code"))?;
        let meta = self.metadata(false).await?;
        let userinfo_url = meta.userinfo_endpoint().map(|u| u.url().clone());
        let client = self.client(meta);
        let tok = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|e| SsoError::token(format!("token endpoint: {e}")))?
            .set_pkce_verifier(PkceCodeVerifier::new(pending.verifier.clone()))
            .request_async(&self.http)
            .await
            .map_err(|e| SsoError::token(format!("token request failed: {}", chain(&e))))?;
        let raw = tok
            .id_token()
            .ok_or_else(|| SsoError::token("the provider sent no ID token"))?
            .to_string();
        let id: IdTok = raw
            .parse()
            .map_err(|e| SsoError::token(format!("malformed ID token: {e}")))?;
        let claims = match self.verify(&client, &id, &pending.nonce, tok.access_token()) {
            Err(ClaimsVerificationError::SignatureVerification(
                SignatureVerificationError::NoMatchingKey,
            )) => {
                // the provider may have rotated its keys since the JWKS was cached
                let meta = self.metadata(true).await?;
                let client = self.client(meta);
                self.verify(&client, &id, &pending.nonce, tok.access_token())
            }
            r => r,
        }
        .map_err(|e| SsoError::token(format!("ID token rejected: {e}")))?;

        let mut profile = Profile {
            issuer: self.issuer.as_str().to_string(),
            subject: claims.subject().as_str().to_string(),
            preferred_username: claims.preferred_username().map(|u| u.as_str().to_string()),
            email: claims.email().map(|e| e.as_str().to_string()),
            name: claims
                .name()
                .and_then(|n| n.get(None))
                .map(|n| n.as_str().to_string()),
            groups: claims
                .additional_claims()
                .groups
                .as_ref()
                .map(groups_of)
                .unwrap_or_default(),
        };
        let groups_missing =
            self.cfg.admin_group.is_some() && claims.additional_claims().groups.is_none();
        let names_missing = profile.preferred_username.is_none()
            && profile.email.is_none()
            && profile.name.is_none();
        if (groups_missing || names_missing)
            && let Some(url) = userinfo_url
        {
            match self.userinfo(&url, tok.access_token().secret()).await {
                Ok(v) => merge_userinfo(&mut profile, &v, groups_missing)?,
                Err(e) => tracing::warn!("single sign-on: userinfo request failed: {e}"),
            }
        }
        Ok(Verified {
            profile,
            return_to: pending.return_to,
            link_user: pending.link_user,
        })
    }

    fn verify(
        &self,
        client: &Client,
        id: &IdTok,
        nonce: &Nonce,
        access_token: &openidconnect::AccessToken,
    ) -> Result<Claims, ClaimsVerificationError> {
        use CoreJwsSigningAlgorithm as A;
        let verifier = client
            .id_token_verifier()
            .set_allowed_algs([
                A::RsaSsaPkcs1V15Sha256,
                A::RsaSsaPkcs1V15Sha384,
                A::RsaSsaPkcs1V15Sha512,
                A::RsaSsaPssSha256,
                A::RsaSsaPssSha384,
                A::RsaSsaPssSha512,
                A::EcdsaP256Sha256,
                A::EcdsaP384Sha384,
                A::EdDsa,
            ])
            // `exp` with leeway for clock skew
            .set_time_fn(|| chrono::Utc::now() - chrono::Duration::seconds(LEEWAY_SECS))
            .set_issue_time_verifier_fn(|iat| {
                let now = chrono::Utc::now();
                if iat > now + chrono::Duration::seconds(LEEWAY_SECS) {
                    Err(format!("issued in the future ({iat})"))
                } else if iat < now - chrono::Duration::seconds(MAX_TOKEN_AGE_SECS) {
                    Err(format!("issued too long ago ({iat})"))
                } else {
                    Ok(())
                }
            });
        let claims = id.claims(&verifier, nonce)?.clone();
        if let Some(expected) = claims.access_token_hash() {
            let alg = id
                .signing_alg()
                .map_err(ClaimsVerificationError::SignatureVerification)?;
            let key = id
                .signing_key(&verifier)
                .map_err(ClaimsVerificationError::SignatureVerification)?;
            let actual = AccessTokenHash::from_token(access_token, alg, key)
                .map_err(|e| ClaimsVerificationError::Other(format!("at_hash: {e}")))?;
            if actual != *expected {
                return Err(ClaimsVerificationError::Other(
                    "access token hash (at_hash) does not match".into(),
                ));
            }
        }
        Ok(claims)
    }

    async fn userinfo(
        &self,
        url: &openidconnect::url::Url,
        token: &str,
    ) -> Result<serde_json::Value, String> {
        let r = self
            .http
            .get(url.clone())
            .bearer_auth(token)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|e| chain(&e))?;
        if !r.status().is_success() {
            return Err(format!("HTTP {}", r.status()));
        }
        let body = r.bytes().await.map_err(|e| chain(&e))?;
        if body.len() > 256 * 1024 {
            return Err("response too large".into());
        }
        serde_json::from_slice(&body).map_err(|e| format!("not JSON: {e}"))
    }
}

/// Adds userinfo claims to `p`; the userinfo `sub` must be the ID token's.
fn merge_userinfo(p: &mut Profile, v: &serde_json::Value, groups: bool) -> Result<(), SsoError> {
    if v.get("sub").and_then(|s| s.as_str()) != Some(p.subject.as_str()) {
        return Err(SsoError::token("userinfo is for another subject"));
    }
    let s = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .filter(|x| !x.trim().is_empty())
    };
    if p.preferred_username.is_none() {
        p.preferred_username = s("preferred_username");
    }
    if p.email.is_none() {
        p.email = s("email");
    }
    if p.name.is_none() {
        p.name = s("name");
    }
    if groups && let Some(g) = v.get("groups") {
        p.groups = groups_of(g);
    }
    Ok(())
}

fn groups_of(v: &serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|g| g.as_str())
            .map(str::to_string)
            .collect(),
        serde_json::Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// An error with its sources (`reqwest` hides the interesting part in them).
fn chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(": ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

#[derive(Debug, Deserialize, Default)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Same-origin relative paths only (`/l/1/authors?x=1`); anything else becomes `/`.
pub fn safe_return(p: Option<&str>) -> String {
    let p = p.unwrap_or("").trim();
    let ok = p.starts_with('/')
        && !p.starts_with("//")
        && p.len() <= 2048
        && !p
            .chars()
            .any(|c| c.is_control() || c == '\\' || c.is_whitespace())
        && !p.starts_with("/api/")
        && !p.starts_with("/opds")
        && !p.starts_with("/login");
    if ok { p.to_string() } else { "/".into() }
}

/// The user a verified sign-in belongs to (created, or linked when `link_user` is set).
pub enum Outcome {
    SignedIn(User),
    Linked(User),
}

/// Finds or creates the account of `v` (or links it to `v.link_user`), and applies the admin
/// group. Blocking (app.db).
pub fn resolve_account(st: &AppState, cfg: &OidcConfig, v: &Verified) -> Result<Outcome, SsoError> {
    let p = &v.profile;
    let c = st.db.lock();
    let dberr = |e: crate::error::ApiError| SsoError::new("internal", e.to_string());
    if let Some(uid) = v.link_user {
        let user = db::get_user(&c, uid)
            .map_err(dberr)?
            .ok_or_else(|| SsoError::new("session", "the account to link is gone"))?;
        db::link_identity(&c, uid, &p.issuer, &p.subject, p.email.as_deref()).map_err(|e| {
            if e.code == "conflict" {
                SsoError::new("already_linked", e.message)
            } else {
                dberr(e)
            }
        })?;
        db::touch_identity(&c, &p.issuer, &p.subject, p.email.as_deref()).map_err(dberr)?;
        tracing::info!(user = %user.username, "single sign-on identity linked");
        return Ok(Outcome::Linked(user));
    }
    let existing = db::identity_user(&c, &p.issuer, &p.subject).map_err(dberr)?;
    let in_admin_group = cfg
        .admin_group
        .as_ref()
        .map(|g| p.groups.iter().any(|x| x == g));
    let mut user = match existing {
        Some(u) => u,
        None => {
            if !cfg.auto_create {
                return Err(SsoError::new(
                    "not_linked",
                    format!(
                        "no account is linked to {} and auto-creation is off",
                        p.subject
                    ),
                ));
            }
            let name = unique_username(&c, p).map_err(dberr)?;
            let role = if in_admin_group == Some(true) {
                "admin"
            } else {
                "reader"
            };
            // no password: `verify_password` never accepts an empty hash
            let u = db::insert_user(&c, &name, "", role).map_err(dberr)?;
            db::link_identity(&c, u.id, &p.issuer, &p.subject, p.email.as_deref())
                .map_err(dberr)?;
            tracing::info!(user = %u.username, role, "account created by single sign-on");
            u
        }
    };
    let mut role_changed = false;
    if let Some(admin) = in_admin_group {
        // the FREELIB_ADMIN_USER account stays an administrator: the way back in
        let bootstrap = st.cfg.admin_password.is_some()
            && user.username.eq_ignore_ascii_case(&st.cfg.admin_user);
        let role = if admin || bootstrap {
            "admin"
        } else {
            "reader"
        };
        if user.role != role {
            db::update_user(&c, user.id, None, Some(role)).map_err(dberr)?;
            tracing::info!(user = %user.username, role, "role set from the admin group");
            user.role = role.into();
            role_changed = true;
        }
    }
    db::touch_identity(&c, &p.issuer, &p.subject, p.email.as_deref()).map_err(dberr)?;
    drop(c);
    if role_changed {
        st.invalidate_sessions();
        let _ = st.events().send(crate::jobs::Event::Users);
    }
    Ok(Outcome::SignedIn(user))
}

/// A free user name from `preferred_username`, `email` or `name` (`alice`, `alice (2)`, …).
fn unique_username(
    c: &rusqlite::Connection,
    p: &Profile,
) -> Result<String, crate::error::ApiError> {
    let clean = |s: &str| -> String {
        let s: String = s
            .chars()
            .filter(|c| !c.is_control() && *c != ':')
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        s.chars().take(56).collect::<String>().trim().to_string()
    };
    let base = [&p.preferred_username, &p.email, &p.name]
        .into_iter()
        .flatten()
        .map(|s| clean(s))
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| {
            format!(
                "user-{}",
                &crate::util::sha256_hex(p.subject.as_bytes())[..8]
            )
        });
    for i in 1..1000 {
        let name = if i == 1 {
            base.clone()
        } else {
            format!("{base} ({i})")
        };
        if db::user_with_hash(c, &name)?.is_none() {
            return Ok(name);
        }
    }
    Ok(format!("user-{}", crate::util::random_token(6)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_paths() {
        assert_eq!(safe_return(Some("/l/1/authors?x=1")), "/l/1/authors?x=1");
        assert_eq!(safe_return(Some("/settings/account")), "/settings/account");
        for bad in [
            "https://evil.example",
            "//evil.example",
            "/\\evil.example",
            "\\\\evil",
            "evil",
            "/x\r\nSet-Cookie: a=b",
            "/api/v1/logout",
            "/login",
            "",
            "javascript:alert(1)",
            "/ /evil",
        ] {
            assert_eq!(safe_return(Some(bad)), "/", "{bad:?}");
        }
        assert_eq!(safe_return(None), "/");
    }

    #[test]
    fn groups_claim_shapes() {
        assert_eq!(
            groups_of(&serde_json::json!(["a", "b", 3])),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(groups_of(&serde_json::json!("a")), vec!["a".to_string()]);
        assert!(groups_of(&serde_json::json!({"a": 1})).is_empty());
    }
}
