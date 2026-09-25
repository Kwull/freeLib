//! Single sign-on (OpenID Connect) against an in-process fake provider: discovery, JWKS with
//! RSA keys generated here, an authorize endpoint that approves at once, and a token endpoint
//! that checks PKCE and issues signed ID tokens.

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};

use axum::Router;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use common::{Resp, TestApp};
use freelib_server::config::OidcConfig;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{RandomizedSigner, SignatureEncoding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, sha2::Sha256};
use serde_json::{Value, json};
use sha2::Digest;

const CLIENT: &str = "freelib-test";
const PUBLIC: &str = "http://books.test:8080";

// ---------------------------------------------------------------- fake provider

fn keys() -> &'static [RsaPrivateKey] {
    static K: OnceLock<Vec<RsaPrivateKey>> = OnceLock::new();
    K.get_or_init(|| {
        let mut rng = rsa::rand_core::OsRng;
        (0..3)
            .map(|_| RsaPrivateKey::new(&mut rng, 2048).expect("RSA key"))
            .collect()
    })
}

/// What the provider does on the next sign-in.
#[derive(Clone)]
struct Knobs {
    sub: String,
    preferred_username: Option<String>,
    email: Option<String>,
    groups: Option<Value>,
    /// Put `groups` into the userinfo response only.
    groups_in_userinfo: bool,
    aud: Option<String>,
    exp_offset: i64,
    iat_offset: i64,
    nonce: Option<String>,
    /// Signing key (index into `keys()`), and the keys the JWKS publishes.
    sign_key: usize,
    jwks: Vec<usize>,
    wrong_at_hash: bool,
}

impl Default for Knobs {
    fn default() -> Self {
        Knobs {
            sub: "sub-alice".into(),
            preferred_username: Some("alice".into()),
            email: Some("alice@example.org".into()),
            groups: None,
            groups_in_userinfo: false,
            aud: None,
            exp_offset: 300,
            iat_offset: 0,
            nonce: None,
            sign_key: 0,
            jwks: vec![0],
            wrong_at_hash: false,
        }
    }
}

struct Issued {
    nonce: Option<String>,
    challenge: String,
    redirect_uri: String,
}

struct Fake {
    base: String,
    knobs: Mutex<Knobs>,
    codes: Mutex<HashMap<String, Issued>>,
    tokens: Mutex<HashMap<String, Knobs>>,
    jwks_fetches: Mutex<u32>,
}

type F = Arc<Fake>;

async fn discovery(State(f): State<F>) -> Response {
    axum::Json(json!({
        "issuer": f.base,
        "authorization_endpoint": format!("{}/authorize", f.base),
        "token_endpoint": format!("{}/token", f.base),
        "jwks_uri": format!("{}/jwks", f.base),
        "userinfo_endpoint": format!("{}/userinfo", f.base),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post", "none"],
        "code_challenge_methods_supported": ["S256"],
    }))
    .into_response()
}

fn jwk(i: usize) -> Value {
    let k = keys()[i].to_public_key();
    json!({
        "kty": "RSA", "use": "sig", "alg": "RS256", "kid": format!("k{i}"),
        "n": B64.encode(k.n().to_bytes_be()),
        "e": B64.encode(k.e().to_bytes_be()),
    })
}

async fn jwks(State(f): State<F>) -> Response {
    *f.jwks_fetches.lock().unwrap() += 1;
    let ks: Vec<Value> = f
        .knobs
        .lock()
        .unwrap()
        .jwks
        .iter()
        .map(|i| jwk(*i))
        .collect();
    axum::Json(json!({ "keys": ks })).into_response()
}

async fn authorize(State(f): State<F>, Query(q): Query<HashMap<String, String>>) -> Response {
    assert_eq!(q.get("response_type").map(String::as_str), Some("code"));
    assert_eq!(q.get("client_id").map(String::as_str), Some(CLIENT));
    assert_eq!(
        q.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert!(q["scope"].split(' ').any(|s| s == "openid"), "{q:?}");
    let code = format!("code-{}", rand_hex());
    f.codes.lock().unwrap().insert(
        code.clone(),
        Issued {
            nonce: q.get("nonce").cloned(),
            challenge: q["code_challenge"].clone(),
            redirect_uri: q["redirect_uri"].clone(),
        },
    );
    let loc = format!(
        "{}?code={code}&state={}",
        q["redirect_uri"],
        urlencode(&q["state"])
    );
    (StatusCode::FOUND, [(header::LOCATION, loc)]).into_response()
}

fn rand_hex() -> String {
    let mut b = [0u8; 12];
    rsa::rand_core::RngCore::fill_bytes(&mut rsa::rand_core::OsRng, &mut b);
    hex::encode(b)
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn sign(key: usize, claims: &Value) -> String {
    let header = json!({"alg": "RS256", "typ": "JWT", "kid": format!("k{key}")});
    let input = format!(
        "{}.{}",
        B64.encode(header.to_string()),
        B64.encode(claims.to_string())
    );
    let sk = SigningKey::<Sha256>::new(keys()[key].clone());
    let sig = sk.sign_with_rng(&mut rsa::rand_core::OsRng, input.as_bytes());
    format!("{input}.{}", B64.encode(sig.to_bytes()))
}

async fn token(State(f): State<F>, Form(p): Form<HashMap<String, String>>) -> Response {
    let bad = |m: &str| {
        (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": "invalid_grant", "error_description": m})),
        )
            .into_response()
    };
    if p.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return bad("grant_type");
    }
    let Some(issued) = f.codes.lock().unwrap().remove(&p["code"]) else {
        return bad("unknown code");
    };
    let verifier = p.get("code_verifier").cloned().unwrap_or_default();
    if B64.encode(sha2::Sha256::digest(verifier.as_bytes())) != issued.challenge {
        return bad("PKCE verification failed");
    }
    if p.get("redirect_uri") != Some(&issued.redirect_uri) {
        return bad("redirect_uri");
    }
    let k = f.knobs.lock().unwrap().clone();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let access = format!("at-{}", rand_hex());
    let digest = sha2::Sha256::digest(if k.wrong_at_hash {
        b"other".to_vec()
    } else {
        access.clone().into_bytes()
    });
    let mut c = json!({
        "iss": f.base,
        "sub": k.sub,
        "aud": k.aud.clone().unwrap_or_else(|| CLIENT.into()),
        "exp": now + k.exp_offset,
        "iat": now + k.iat_offset,
        "nonce": k.nonce.clone().or(issued.nonce),
        "at_hash": B64.encode(&digest[..16]),
    });
    if let Some(u) = &k.preferred_username {
        c["preferred_username"] = json!(u);
    }
    if let Some(e) = &k.email {
        c["email"] = json!(e);
    }
    if let Some(g) = &k.groups
        && !k.groups_in_userinfo
    {
        c["groups"] = g.clone();
    }
    f.tokens.lock().unwrap().insert(access.clone(), k.clone());
    axum::Json(json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": sign(k.sign_key, &c),
    }))
    .into_response()
}

async fn userinfo(State(f): State<F>, headers: HeaderMap) -> Response {
    let tok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    let Some(k) = f.tokens.lock().unwrap().get(tok).cloned() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let mut v =
        json!({ "sub": k.sub, "preferred_username": k.preferred_username, "email": k.email });
    if let Some(g) = k.groups {
        v["groups"] = g;
    }
    axum::Json(v).into_response()
}

async fn start_fake() -> F {
    // key generation is slow: never inside a request the server under test waits on
    tokio::task::spawn_blocking(keys).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let f = Arc::new(Fake {
        base: format!("http://{addr}"),
        knobs: Mutex::new(Knobs::default()),
        codes: Mutex::new(HashMap::new()),
        tokens: Mutex::new(HashMap::new()),
        jwks_fetches: Mutex::new(0),
    });
    let app = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/userinfo", get(userinfo))
        .with_state(f.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    f
}

impl Fake {
    fn set(&self, f: impl FnOnce(&mut Knobs)) {
        f(&mut self.knobs.lock().unwrap());
    }
}

// ---------------------------------------------------------------- browser side

async fn app_with(
    f: &Fake,
    tweak: impl FnOnce(&mut OidcConfig, &mut freelib_server::Config),
) -> TestApp {
    let issuer = f.base.clone();
    TestApp::new(move |cfg, _| {
        cfg.public_url = Some(PUBLIC.into());
        let mut o = OidcConfig::new(&issuer, CLIENT);
        o.admin_group = None;
        tweak(&mut o, cfg);
        cfg.oidc = Some(o);
    })
    .await
}

fn cookie_of(r: &Resp, name: &str) -> Option<String> {
    r.headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with(&format!("{name}=")))
        .map(|c| c.to_string())
}

fn pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().unwrap().to_string()
}

/// One browser: its cookies (name → "name=value").
#[derive(Default, Clone)]
struct Browser {
    cookies: HashMap<String, String>,
}

impl Browser {
    fn header(&self) -> Option<String> {
        (!self.cookies.is_empty()).then(|| {
            self.cookies
                .values()
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        })
    }
    fn absorb(&mut self, r: &Resp) {
        for c in r
            .headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
        {
            let p = pair(c);
            let (name, value) = p.split_once('=').unwrap();
            if value.is_empty() || c.contains("Max-Age=0") {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_string(), p.clone());
            }
        }
    }
    async fn send(
        &mut self,
        app: &TestApp,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Resp {
        let mut b = axum::http::Request::builder().method(method).uri(path);
        if let Some(c) = self.header() {
            b = b.header(header::COOKIE, c);
        }
        let req = match body {
            Some(v) => b
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(v.to_string()))
                .unwrap(),
            None => b.body(axum::body::Body::empty()).unwrap(),
        };
        let r = app.send(req).await;
        self.absorb(&r);
        r
    }
    async fn get(&mut self, app: &TestApp, path: &str) -> Resp {
        self.send(app, Method::GET, path, None).await
    }
}

/// The provider's redirect back to us, as a path: follows the authorize URL.
async fn approve(authorize_url: &str) -> String {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let r = http.get(authorize_url).send().await.unwrap();
    assert_eq!(r.status(), 302, "{:?}", r.text().await);
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    let path = loc
        .strip_prefix(PUBLIC)
        .expect("redirect_uri under the public URL");
    assert!(path.starts_with("/api/v1/auth/oidc/callback?"), "{path}");
    path.to_string()
}

/// Full sign-in; returns the callback response (a redirect).
async fn sso(app: &TestApp, b: &mut Browser, ret: &str) -> Resp {
    let r = b
        .get(
            app,
            &format!("/api/v1/auth/oidc/login?return={}", urlencode(ret)),
        )
        .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.text());
    let loc = r.header("location");
    assert!(cookie_of(&r, "freelib_oidc").is_some());
    let cb = approve(&loc).await;
    b.get(app, &cb).await
}

async fn whoami(app: &TestApp, b: &mut Browser) -> Value {
    b.get(app, "/api/v1/session").await.json()["user"].clone()
}

// ---------------------------------------------------------------- tests

#[tokio::test]
async fn sign_in_creates_reader_and_reuses_account() {
    let f = start_fake().await;
    let app = app_with(&f, |_, _| {}).await;

    let s = app.get("/api/v1/session").await.json();
    assert_eq!(s["openMode"], false, "single sign-on is never open mode");
    assert_eq!(s["auth"]["password"], true);
    assert_eq!(s["auth"]["oidc"]["label"], "Sign in with SSO");

    let r = app.get("/api/v1/auth/oidc/login?return=/l/1/authors").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let loc = r.header("location");
    assert!(loc.starts_with(&format!("{}/authorize?", f.base)), "{loc}");
    for p in [
        "code_challenge_method=S256",
        "state=",
        "nonce=",
        "code_challenge=",
    ] {
        assert!(loc.contains(p), "{p} in {loc}");
    }
    assert!(
        loc.contains(
            "redirect_uri=http%3A%2F%2Fbooks.test%3A8080%2Fapi%2Fv1%2Fauth%2Foidc%2Fcallback&"
        ),
        "{loc}"
    );
    let sc = cookie_of(&r, "freelib_oidc").unwrap();
    assert!(
        sc.contains("HttpOnly")
            && sc.contains("SameSite=Lax")
            && sc.contains("Path=/api/v1/auth/oidc"),
        "{sc}"
    );
    assert!(!sc.contains("Secure"), "http public URL: no Secure flag");

    let mut b = Browser::default();
    let r = sso(&app, &mut b, "/l/1/authors?x=1").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.text());
    assert_eq!(r.header("location"), "/l/1/authors?x=1");
    let session = cookie_of(&r, "freelib_session").expect("session cookie");
    assert!(session.contains("HttpOnly"));
    assert!(
        !b.cookies.contains_key("freelib_oidc"),
        "state cookie cleared"
    );
    let u = whoami(&app, &mut b).await;
    assert_eq!(u["username"], "alice");
    assert_eq!(u["role"], "reader");
    let acc = b.get(&app, "/api/v1/me/account").await.json();
    assert_eq!(acc["hasPassword"], false);
    assert_eq!(acc["sso"]["linked"], true);
    assert_eq!(acc["sso"]["email"], "alice@example.org");

    // a second sign-in: same account, new session id
    let old = b.cookies["freelib_session"].clone();
    let mut b2 = b.clone();
    let r = sso(&app, &mut b2, "/").await;
    assert_eq!(r.header("location"), "/");
    assert_ne!(b2.cookies["freelib_session"], old, "session id rotated");
    assert_eq!(whoami(&app, &mut b2).await["id"], u["id"]);
    // the previous session of this browser is gone
    let mut stale = Browser::default();
    stale.cookies.insert("freelib_session".into(), old);
    assert!(whoami(&app, &mut stale).await.is_null());

    // SSO accounts cannot sign in with a password (they have none)
    let r = app
        .post(
            "/api/v1/login",
            &json!({"username": "alice", "password": ""}),
        )
        .await;
    assert_ne!(r.status, StatusCode::OK);

    let c = app.state.db.lock();
    assert_eq!(freelib_server::db::count_users(&c).unwrap(), 1);
}

#[tokio::test]
async fn username_is_deduplicated_never_merged() {
    let f = start_fake().await;
    let app = app_with(&f, |_, cfg| cfg.admin_password = Some("secret".into())).await;
    f.set(|k| {
        k.preferred_username = Some("Admin".into());
        k.sub = "sub-mallory".into();
    });
    let mut b = Browser::default();
    let r = sso(&app, &mut b, "/").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let u = whoami(&app, &mut b).await;
    assert_eq!(u["username"], "Admin (2)");
    assert_eq!(u["role"], "reader");
}

#[tokio::test]
async fn auto_create_off_needs_a_linked_account() {
    let f = start_fake().await;
    let app = app_with(&f, |o, _| o.auto_create = false).await;
    let mut b = Browser::default();
    let r = sso(&app, &mut b, "/").await;
    assert_eq!(r.header("location"), "/login?ssoError=not_linked");
    assert!(cookie_of(&r, "freelib_session").is_none());
    assert!(whoami(&app, &mut b).await.is_null());
    let c = app.state.db.lock();
    assert_eq!(freelib_server::db::count_users(&c).unwrap(), 0);
}

#[tokio::test]
async fn admin_group_is_applied_at_every_sign_in() {
    let f = start_fake().await;
    let app = app_with(&f, |o, _| o.admin_group = Some("freelib-admins".into())).await;
    f.set(|k| k.groups = Some(json!(["users", "freelib-admins"])));
    let mut b = Browser::default();
    sso(&app, &mut b, "/").await;
    assert_eq!(whoami(&app, &mut b).await["role"], "admin");
    assert_eq!(b.get(&app, "/api/v1/users").await.status, StatusCode::OK);

    // removed from the group: a reader at the next sign-in
    f.set(|k| k.groups = Some(json!(["users"])));
    let mut b2 = Browser::default();
    sso(&app, &mut b2, "/").await;
    assert_eq!(whoami(&app, &mut b2).await["role"], "reader");
    // the old browser's session sees the demotion too
    assert_eq!(
        b.get(&app, "/api/v1/users").await.status,
        StatusCode::FORBIDDEN
    );

    // groups only in userinfo (Authelia): still found
    f.set(|k| {
        k.groups = Some(json!("freelib-admins"));
        k.groups_in_userinfo = true;
    });
    let mut b3 = Browser::default();
    sso(&app, &mut b3, "/").await;
    assert_eq!(whoami(&app, &mut b3).await["role"], "admin");
}

#[tokio::test]
async fn state_must_match_the_cookie_and_is_single_use() {
    let f = start_fake().await;
    let app = app_with(&f, |_, _| {}).await;

    // callback without starting a sign-in in this browser (login CSRF)
    let mut victim = Browser::default();
    let mut attacker = Browser::default();
    let r = attacker.get(&app, "/api/v1/auth/oidc/login").await;
    let cb = approve(&r.header("location")).await;
    let r = victim.get(&app, &cb).await;
    assert_eq!(r.header("location"), "/login?ssoError=state");
    assert!(whoami(&app, &mut victim).await.is_null());

    // right cookie, tampered state
    let mut b = Browser::default();
    let r = b.get(&app, "/api/v1/auth/oidc/login").await;
    let cb = approve(&r.header("location")).await;
    let tampered = cb.replace("state=", "state=x");
    let r = b.get(&app, &tampered).await;
    assert_eq!(r.header("location"), "/login?ssoError=state");

    // the genuine callback still works once, a replay does not
    let mut b = Browser::default();
    let r = b.get(&app, "/api/v1/auth/oidc/login").await;
    let state_cookie = b.cookies["freelib_oidc"].clone();
    let cb = approve(&r.header("location")).await;
    let r = b.get(&app, &cb).await;
    assert_eq!(r.header("location"), "/");
    let mut replay = Browser::default();
    replay.cookies.insert("freelib_oidc".into(), state_cookie);
    let r = replay.get(&app, &cb).await;
    assert_eq!(r.header("location"), "/login?ssoError=state");
}

#[tokio::test]
async fn provider_error_is_reported() {
    let f = start_fake().await;
    let app = app_with(&f, |_, _| {}).await;
    let mut b = Browser::default();
    let r = b.get(&app, "/api/v1/auth/oidc/login").await;
    let loc = r.header("location");
    let state = loc
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();
    let r = b
        .get(
            &app,
            &format!("/api/v1/auth/oidc/callback?error=access_denied&error_description=nope&state={state}"),
        )
        .await;
    assert_eq!(r.header("location"), "/login?ssoError=provider");
}

async fn rejected(tweak: impl FnOnce(&mut Knobs)) {
    let f = start_fake().await;
    let app = app_with(&f, |_, _| {}).await;
    f.set(tweak);
    let mut b = Browser::default();
    let r = sso(&app, &mut b, "/").await;
    assert_eq!(r.header("location"), "/login?ssoError=token");
    assert!(cookie_of(&r, "freelib_session").is_none());
    assert!(whoami(&app, &mut b).await.is_null());
}

#[tokio::test]
async fn bad_nonce_is_rejected() {
    rejected(|k| k.nonce = Some("not-the-nonce".into())).await;
}

#[tokio::test]
async fn wrong_audience_is_rejected() {
    rejected(|k| k.aud = Some("another-client".into())).await;
}

#[tokio::test]
async fn expired_token_is_rejected() {
    // beyond the 60 s leeway
    rejected(|k| {
        k.exp_offset = -120;
        k.iat_offset = -400;
    })
    .await;
}

#[tokio::test]
async fn token_from_the_future_is_rejected() {
    rejected(|k| k.iat_offset = 600).await;
}

#[tokio::test]
async fn unknown_signing_key_is_rejected() {
    // signed with a key the provider does not publish (a forgery)
    rejected(|k| {
        k.sign_key = 2;
        k.jwks = vec![0];
    })
    .await;
}

#[tokio::test]
async fn substituted_access_token_is_rejected() {
    rejected(|k| k.wrong_at_hash = true).await;
}

#[tokio::test]
async fn small_clock_skew_is_accepted() {
    let f = start_fake().await;
    let app = app_with(&f, |_, _| {}).await;
    f.set(|k| {
        k.exp_offset = -30;
        k.iat_offset = 30;
    });
    let mut b = Browser::default();
    let r = sso(&app, &mut b, "/").await;
    assert_eq!(r.header("location"), "/");
}

#[tokio::test]
async fn key_rotation_refetches_the_jwks() {
    let f = start_fake().await;
    let app = app_with(&f, |_, _| {}).await;
    let mut b = Browser::default();
    assert_eq!(sso(&app, &mut b, "/").await.header("location"), "/");
    let before = *f.jwks_fetches.lock().unwrap();
    f.set(|k| {
        k.sign_key = 1;
        k.jwks = vec![0, 1];
    });
    let mut b = Browser::default();
    assert_eq!(sso(&app, &mut b, "/").await.header("location"), "/");
    assert!(*f.jwks_fetches.lock().unwrap() > before, "JWKS refetched");
}

#[tokio::test]
async fn open_redirects_are_refused() {
    let f = start_fake().await;
    let app = app_with(&f, |_, _| {}).await;
    for evil in [
        "https://evil.example/",
        "//evil.example/x",
        "/\\evil.example",
        "javascript:alert(1)",
    ] {
        let mut b = Browser::default();
        let r = sso(&app, &mut b, evil).await;
        assert_eq!(r.header("location"), "/", "{evil}");
    }
}

#[tokio::test]
async fn link_and_unlink_a_local_account() {
    let f = start_fake().await;
    let mut app = app_with(&f, |_, cfg| cfg.admin_password = Some("secret".into())).await;
    app.login("admin", "secret").await;
    let r = app
        .post(
            "/api/v1/users",
            &json!({"username": "bob", "password": "bobpass", "role": "reader"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());

    // bob signs in with his password and links the provider identity
    let mut bob = Browser::default();
    let r = bob
        .send(
            &app,
            Method::POST,
            "/api/v1/login",
            Some(&json!({"username": "bob", "password": "bobpass"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let acc = bob.get(&app, "/api/v1/me/account").await.json();
    assert_eq!(acc["hasPassword"], true);
    assert_eq!(acc["sso"]["linked"], false);
    let r = bob
        .send(&app, Method::POST, "/api/v1/auth/oidc/link", None)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let url = r.json()["url"].as_str().unwrap().to_string();
    f.set(|k| {
        k.sub = "sub-bob".into();
        k.preferred_username = Some("robert".into());
    });
    let old_session = bob.cookies["freelib_session"].clone();
    let cb = approve(&url).await;
    let r = bob.get(&app, &cb).await;
    assert_eq!(r.header("location"), "/settings/account?sso=linked");
    assert_ne!(
        bob.cookies["freelib_session"], old_session,
        "new session id after linking"
    );
    assert_eq!(whoami(&app, &mut bob).await["username"], "bob");
    assert_eq!(
        bob.get(&app, "/api/v1/me/account").await.json()["sso"]["linked"],
        true
    );

    // a fresh browser signing in with that identity is bob
    let mut other = Browser::default();
    sso(&app, &mut other, "/").await;
    assert_eq!(whoami(&app, &mut other).await["username"], "bob");

    // the admin sees the link
    let users = app.get("/api/v1/users").await.json();
    let row = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "bob")
        .unwrap()
        .clone();
    assert_eq!(row["hasPassword"], true);
    assert!(row["sso"].is_object(), "{row}");

    // a link flow finished in a browser signed in as someone else is refused
    let r = bob
        .send(&app, Method::POST, "/api/v1/auth/oidc/link", None)
        .await;
    let url = r.json()["url"].as_str().unwrap().to_string();
    let cb = approve(&url).await;
    let mut switched = Browser::default();
    switched
        .cookies
        .insert("freelib_oidc".into(), bob.cookies["freelib_oidc"].clone());
    let r = switched.get(&app, &cb).await;
    assert_eq!(r.header("location"), "/settings/account?ssoError=session");

    // unlink
    let r = bob
        .send(&app, Method::DELETE, "/api/v1/me/oidc", None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.text());
    assert_eq!(
        bob.get(&app, "/api/v1/me/account").await.json()["sso"]["linked"],
        false
    );
    // the identity now makes a new account
    let mut again = Browser::default();
    sso(&app, &mut again, "/").await;
    assert_eq!(whoami(&app, &mut again).await["username"], "robert");

    // robert (SSO only) cannot unlink before setting a password
    let r = again
        .send(&app, Method::DELETE, "/api/v1/me/oidc", None)
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    // …sets one (for OPDS apps) without a current password, keeps his session
    let r = again
        .send(
            &app,
            Method::PUT,
            "/api/v1/me/password",
            Some(&json!({"password": "robpass"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT, "{}", r.text());
    assert_eq!(whoami(&app, &mut again).await["username"], "robert");
    assert_eq!(
        again.get(&app, "/api/v1/me/account").await.json()["hasPassword"],
        true
    );
    // now a current password is required to change it
    let r = again
        .send(
            &app,
            Method::PUT,
            "/api/v1/me/password",
            Some(&json!({"password": "x2345", "current": "wrong"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // OPDS takes the local password
    let opds = |cred: &str| {
        axum::http::Request::builder()
            .uri("/opds/")
            .header(
                header::AUTHORIZATION,
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(cred)
                ),
            )
            .body(axum::body::Body::empty())
            .unwrap()
    };
    assert_eq!(
        app.send(opds("robert:nope")).await.status,
        StatusCode::UNAUTHORIZED
    );
    let r = app.send(opds("robert:robpass")).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    // and unlinking works
    let r = again
        .send(&app, Method::DELETE, "/api/v1/me/oidc", None)
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn password_login_can_be_disabled() {
    let f = start_fake().await;
    let mut app = app_with(&f, |o, cfg| {
        o.disable_password = true;
        cfg.admin_password = Some("secret".into());
    })
    .await;
    let s = app.get("/api/v1/session").await.json();
    assert_eq!(s["auth"]["password"], false);
    // the bootstrap admin keeps password sign-in (the way back in)
    assert_eq!(app.login("admin", "secret").await.status, StatusCode::OK);
    let r = app
        .post(
            "/api/v1/users",
            &json!({"username": "carol", "password": "carolpw", "role": "admin"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = app
        .post(
            "/api/v1/login",
            &json!({"username": "carol", "password": "carolpw"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    // single sign-on still works
    let mut b = Browser::default();
    assert_eq!(sso(&app, &mut b, "/").await.header("location"), "/");
    // and cannot be unlinked (it is the only way in)
    let r = b
        .send(
            &app,
            Method::PUT,
            "/api/v1/me/password",
            Some(&json!({"password": "alicepw"})),
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    let r = b.send(&app, Method::DELETE, "/api/v1/me/oidc", None).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn https_public_url_sets_secure_cookies() {
    let f = start_fake().await;
    let app = TestApp::new({
        let issuer = f.base.clone();
        move |cfg, _| {
            cfg.public_url = Some("https://books.test".into());
            cfg.oidc = Some(OidcConfig::new(&issuer, CLIENT));
        }
    })
    .await;
    let r = app.get("/api/v1/auth/oidc/login").await;
    assert!(cookie_of(&r, "freelib_oidc").unwrap().contains("; Secure"));
    let loc = r.header("location");
    assert!(
        loc.contains("redirect_uri=https%3A%2F%2Fbooks.test%2Fapi%2Fv1%2Fauth%2Foidc%2Fcallback&"),
        "{loc}"
    );
}

#[tokio::test]
async fn unreachable_provider_and_missing_config() {
    // nothing listens on port 9: the login page gets a readable error
    let app = TestApp::new(|cfg, _| {
        cfg.public_url = Some(PUBLIC.into());
        cfg.oidc = Some(OidcConfig::new("http://127.0.0.1:9", CLIENT));
    })
    .await;
    let r = app.get("/api/v1/auth/oidc/login").await;
    assert_eq!(r.header("location"), "/login?ssoError=unavailable");

    // no public URL: the server refuses to start
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = freelib_server::Config::for_dir(dir.path());
    cfg.oidc = Some(OidcConfig::new("http://127.0.0.1:9", CLIENT));
    let err = freelib_server::init(cfg)
        .await
        .err()
        .expect("startup error");
    assert!(err.to_string().contains("FREELIB_PUBLIC_URL"), "{err}");

    // not configured: 404, and /session says so
    let app = TestApp::new(|_, _| {}).await;
    assert_eq!(
        app.get("/api/v1/auth/oidc/login").await.status,
        StatusCode::NOT_FOUND
    );
    assert!(app.get("/api/v1/session").await.json()["auth"]["oidc"].is_null());
}

#[tokio::test]
async fn one_client_cannot_fill_the_pending_table() {
    let f = start_fake().await;
    let app = app_with(&f, |_, _| {}).await;
    let mut first = Browser::default();
    let r = first.get(&app, "/api/v1/auth/oidc/login").await;
    let first_cb = approve(&r.header("location")).await;
    // the same address starts many more sign-ins: its oldest ones are dropped
    for _ in 0..25 {
        app.get("/api/v1/auth/oidc/login").await;
    }
    let r = first.get(&app, &first_cb).await;
    assert_eq!(r.header("location"), "/login?ssoError=state");
    let mut b = Browser::default();
    assert_eq!(sso(&app, &mut b, "/").await.header("location"), "/");
}
