//! OAuth for the MCP endpoint: metadata documents, dynamic registration and Client ID Metadata
//! Documents, authorization with PKCE and consent, tokens, refresh rotation and reuse
//! detection, revocation, audience binding, scopes, expiry, redirect URI attacks, limits.
//! Also: secrets encrypted at rest (SMTP password).

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use base64::Engine;
use common::{Resp, TestApp};
use freelib_server::oauth::Url;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const PW: &str = "admin-pass";
const PUBLIC: &str = "https://books.example.org";
const RESOURCE: &str = "https://books.example.org/mcp";
const LOOPBACK: &str = "http://127.0.0.1/callback";
const CIMD_ID: &str = "https://claude.ai/oauth/test-client-metadata";
const CLAUDE_CB: &str = "https://claude.ai/api/mcp/auth_callback";

fn cimd_doc(id: &str) -> String {
    json!({
        "client_id": id,
        "client_name": "Claude (test)",
        "redirect_uris": [CLAUDE_CB],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    })
    .to_string()
}

async fn app() -> TestApp {
    app_with(|_| {}).await
}

async fn app_with(f: impl FnOnce(&mut freelib_server::Config)) -> TestApp {
    let mut app = TestApp::new(|cfg, _| {
        cfg.admin_password = Some(PW.into());
        cfg.public_url = Some(PUBLIC.into());
        cfg.oauth_client_docs = vec![
            (CIMD_ID.into(), cimd_doc(CIMD_ID)),
            (
                "https://claude.ai/oauth/wrong-id".into(),
                cimd_doc("https://claude.ai/oauth/other"),
            ),
            (
                "https://evil.example/client.json".into(),
                cimd_doc("https://evil.example/client.json"),
            ),
        ];
        f(cfg);
    })
    .await;
    assert_eq!(app.login("admin", PW).await.status, StatusCode::OK);
    app
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut u = Url::parse("http://x/").unwrap();
    u.query_pairs_mut().extend_pairs(pairs);
    u.query().unwrap_or("").to_string()
}

async fn post_form(app: &TestApp, path: &str, pairs: &[(&str, &str)]) -> Resp {
    app.send(
        Request::builder()
            .method(Method::POST)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form(pairs)))
            .unwrap(),
    )
    .await
}

/// A request without the session cookie (what an OAuth client sends).
async fn anon(app: &TestApp, method: Method, path: &str, body: Option<&Value>) -> Resp {
    let mut b = Request::builder().method(method).uri(path);
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    app.send(b.body(body).unwrap()).await
}

async fn register(app: &TestApp, uris: &[&str]) -> Resp {
    anon(
        app,
        Method::POST,
        "/oauth/register",
        Some(&json!({"client_name": "Test client", "redirect_uris": uris,
                     "grant_types": ["authorization_code", "refresh_token"],
                     "token_endpoint_auth_method": "none"})),
    )
    .await
}

async fn register_ok(app: &TestApp) -> String {
    let r = register(app, &[LOOPBACK]).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    r.json()["client_id"].as_str().unwrap().to_string()
}

struct Pkce {
    verifier: String,
    challenge: String,
}

fn pkce() -> Pkce {
    let mut b = [0u8; 32];
    rand::fill(&mut b[..]);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    Pkce {
        verifier,
        challenge,
    }
}

fn authorize_url(client_id: &str, redirect: &str, p: &Pkce, extra: &[(&str, &str)]) -> String {
    let mut pairs = vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect),
        ("code_challenge", p.challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", "st-123"),
        ("resource", RESOURCE),
    ];
    pairs.extend_from_slice(extra);
    format!("/oauth/authorize?{}", form(&pairs))
}

/// `GET /oauth/authorize`: the Location it redirects to.
async fn authorize(app: &TestApp, url: &str) -> String {
    let r = anon(app, Method::GET, url, None).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.text());
    assert_eq!(r.header("cache-control"), "no-store");
    r.header("location")
}

fn query_of(url: &str) -> std::collections::HashMap<String, String> {
    let u = Url::parse(url).unwrap_or_else(|_| Url::parse(&format!("http://x{url}")).unwrap());
    u.query_pairs().into_owned().collect()
}

/// Runs authorize + consent; returns the redirect back to the client.
async fn consent(app: &TestApp, url: &str, scopes: &[&str]) -> String {
    let loc = authorize(app, url).await;
    assert!(loc.starts_with("/oauth/consent?request="), "{loc}");
    let id = query_of(&loc)["request"].clone();
    let v = app.get(&format!("/api/v1/oauth/requests/{id}")).await;
    assert_eq!(v.status, StatusCode::OK, "{}", v.text());
    let v = v.json();
    let r = app
        .post(
            &format!("/api/v1/oauth/requests/{id}"),
            &json!({"approve": true, "scopes": scopes, "csrf": v["csrf"]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    r.json()["redirect"].as_str().unwrap().to_string()
}

async fn exchange(
    app: &TestApp,
    client_id: &str,
    redirect: &str,
    code: &str,
    verifier: &str,
) -> Resp {
    post_form(
        app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect),
            ("client_id", client_id),
            ("code_verifier", verifier),
            ("resource", RESOURCE),
        ],
    )
    .await
}

struct Tokens {
    access: String,
    refresh: String,
    scope: String,
}

/// The full flow for a fresh client: (client id, tokens).
async fn connect(app: &TestApp, scopes: &[&str]) -> (String, Tokens) {
    let client = register_ok(app).await;
    let p = pkce();
    let redirect = "http://127.0.0.1:43210/callback";
    let back = consent(
        app,
        &authorize_url(&client, redirect, &p, &[("scope", "read write send")]),
        scopes,
    )
    .await;
    let q = query_of(&back);
    let r = exchange(app, &client, redirect, &q["code"], &p.verifier).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let v = r.json();
    (
        client,
        Tokens {
            access: v["access_token"].as_str().unwrap().into(),
            refresh: v["refresh_token"].as_str().unwrap().into(),
            scope: v["scope"].as_str().unwrap().into(),
        },
    )
}

async fn mcp(app: &TestApp, token: Option<&str>, method: &str, params: Value) -> Resp {
    let mut b = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, "books.example.org")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25");
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    app.send(b.body(Body::from(body.to_string())).unwrap())
        .await
}

async fn tool_names(app: &TestApp, token: &str) -> Vec<String> {
    let r = mcp(app, Some(token), "tools/list", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    r.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn metadata_documents_and_challenge() {
    let app = app().await;
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let r = anon(&app, Method::GET, path, None).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        let v = r.json();
        assert_eq!(v["resource"], RESOURCE);
        assert_eq!(v["authorization_servers"], json!([PUBLIC]));
        assert_eq!(v["scopes_supported"], json!(["read", "write", "send"]));
        assert_eq!(v["bearer_methods_supported"], json!(["header"]));
    }
    let v = anon(
        &app,
        Method::GET,
        "/.well-known/oauth-authorization-server",
        None,
    )
    .await
    .json();
    assert_eq!(v["issuer"], PUBLIC);
    assert_eq!(
        v["authorization_endpoint"],
        format!("{PUBLIC}/oauth/authorize")
    );
    assert_eq!(v["token_endpoint"], format!("{PUBLIC}/oauth/token"));
    assert_eq!(
        v["registration_endpoint"],
        format!("{PUBLIC}/oauth/register")
    );
    assert_eq!(v["revocation_endpoint"], format!("{PUBLIC}/oauth/revoke"));
    assert_eq!(v["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(v["token_endpoint_auth_methods_supported"], json!(["none"]));
    assert_eq!(v["client_id_metadata_document_supported"], true);
    assert_eq!(v["authorization_response_iss_parameter_supported"], true);
    assert_eq!(
        v["grant_types_supported"],
        json!(["authorization_code", "refresh_token"])
    );

    // /mcp without a token: 401 pointing at the resource metadata
    let r = mcp(&app, None, "tools/list", json!({})).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let h = r.header("www-authenticate");
    assert!(
        h.contains(&format!(
            "resource_metadata=\"{PUBLIC}/.well-known/oauth-protected-resource/mcp\""
        )),
        "{h}"
    );
    assert!(h.contains("scope=\"read write send\""), "{h}");
    assert!(!h.contains("invalid_token"), "{h}");
    let r = mcp(
        &app,
        Some("flo_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        "tools/list",
        json!({}),
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(
        r.header("www-authenticate")
            .contains("error=\"invalid_token\"")
    );

    // tokens page reports OAuth
    let t = app.get("/api/v1/me/tokens").await.json();
    assert_eq!(t["mcp"]["oauth"], true);
    assert_eq!(t["mcp"]["url"], RESOURCE);
}

#[tokio::test]
async fn off_without_https_public_url() {
    for url in [
        None,
        Some("http://books.example.org"),
        Some("https://books.example.org/sub"),
    ] {
        let app = TestApp::new(|cfg, _| {
            cfg.admin_password = Some(PW.into());
            cfg.public_url = url.map(String::from);
        })
        .await;
        for path in [
            "/.well-known/oauth-protected-resource/mcp",
            "/.well-known/oauth-authorization-server",
        ] {
            assert_eq!(
                anon(&app, Method::GET, path, None).await.status,
                StatusCode::NOT_FOUND,
                "{url:?} {path}"
            );
        }
        assert_eq!(
            register(&app, &[LOOPBACK]).await.status,
            StatusCode::NOT_FOUND
        );
        let r = mcp(&app, None, "tools/list", json!({})).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
        assert!(!r.header("www-authenticate").contains("resource_metadata"));
        assert!(
            authorize(&app, "/oauth/authorize?client_id=x")
                .await
                .ends_with("error=disabled")
        );
    }
}

#[tokio::test]
async fn full_flow_refresh_rotation_reuse_and_revoke() {
    let app = app().await;
    let client = register_ok(&app).await;
    assert!(client.starts_with("flc_"), "{client}");
    let p = pkce();
    let redirect = "http://127.0.0.1:43210/callback"; // registered without a port: any port
    let loc = authorize(
        &app,
        &authorize_url(&client, redirect, &p, &[("scope", "read write")]),
    )
    .await;
    let id = query_of(&loc)["request"].clone();

    // the consent page needs a signed-in user
    let r = anon(
        &app,
        Method::GET,
        &format!("/api/v1/oauth/requests/{id}"),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let v = app
        .get(&format!("/api/v1/oauth/requests/{id}"))
        .await
        .json();
    assert_eq!(v["client"]["name"], "Test client");
    assert_eq!(v["client"]["kind"], "dcr");
    assert_eq!(v["redirectHost"], "127.0.0.1");
    assert_eq!(v["loopback"], true);
    assert_eq!(v["scopes"], json!(["read", "write"]));
    assert_eq!(v["resource"], RESOURCE);
    // CSRF token required
    let r = app
        .post(
            &format!("/api/v1/oauth/requests/{id}"),
            &json!({"approve": true, "scopes": ["read"], "csrf": "forged"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // a scope that was not requested
    let r = app
        .post(
            &format!("/api/v1/oauth/requests/{id}"),
            &json!({"approve": true, "scopes": ["send"], "csrf": v["csrf"]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // cross-site POST refused by the API's CSRF layer
    let mut req = app.request(
        Method::POST,
        &format!("/api/v1/oauth/requests/{id}"),
        Some(&json!({"approve": true, "scopes": ["read"], "csrf": v["csrf"]})),
    );
    req.headers_mut()
        .insert("sec-fetch-site", "cross-site".parse().unwrap());
    assert_eq!(app.send(req).await.status, StatusCode::FORBIDDEN);
    // the user narrows the scopes to read
    let r = app
        .post(
            &format!("/api/v1/oauth/requests/{id}"),
            &json!({"approve": true, "scopes": ["read"], "csrf": v["csrf"]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let back = r.json()["redirect"].as_str().unwrap().to_string();
    assert!(
        back.starts_with("http://127.0.0.1:43210/callback?"),
        "{back}"
    );
    let q = query_of(&back);
    assert_eq!(q["state"], "st-123");
    assert_eq!(q["iss"], PUBLIC);
    let code = q["code"].clone();
    // single use request
    let r = app
        .post(
            &format!("/api/v1/oauth/requests/{id}"),
            &json!({"approve": true, "scopes": ["read"], "csrf": v["csrf"]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);

    // token: PKCE, redirect URI and client must match
    let r = exchange(&app, &client, redirect, &code, &pkce().verifier).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], "invalid_grant");
    let r = exchange(&app, &client, redirect, &code, &p.verifier).await;
    // the failed attempt did not burn the code
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.header("cache-control"), "no-store");
    let t = r.json();
    assert_eq!(t["token_type"], "Bearer");
    assert_eq!(t["expires_in"], 3600);
    assert_eq!(t["scope"], "read");
    let access = t["access_token"].as_str().unwrap().to_string();
    let refresh = t["refresh_token"].as_str().unwrap().to_string();
    assert!(access.starts_with("flo_") && refresh.starts_with("flr_"));
    // secrets are not stored
    {
        let c = app.state.db.lock();
        let n: i64 = c
            .query_row(
                "SELECT count(*) FROM oauth_token WHERE token_hash IN (?1, ?2)",
                [&access, &refresh],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }

    // MCP with the access token: read tools only; a send tool gets a step-up challenge
    let names = tool_names(&app, &access).await;
    assert!(names.contains(&"search_books".to_string()));
    assert!(!names.contains(&"send_books".to_string()));
    assert!(!names.contains(&"rate_book".to_string()));
    let r = mcp(
        &app,
        Some(&access),
        "tools/call",
        json!({"name": "send_books", "arguments": {"book_ids": [1], "device_id": 1}}),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    let h = r.header("www-authenticate");
    assert!(h.contains("error=\"insufficient_scope\""), "{h}");
    assert!(h.contains("scope=\"read send\""), "{h}");
    assert!(h.contains("resource_metadata="), "{h}");
    let r = mcp(
        &app,
        Some(&access),
        "tools/call",
        json!({"name": "list_libraries", "arguments": {}}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());

    // the audit log names the app
    let audit = app.get("/api/v1/me/tokens/audit").await.json();
    let rows = audit.as_array().unwrap();
    assert!(
        rows.iter()
            .any(|a| a["tool"] == "list_libraries" && a["appName"] == "Test client"),
        "{audit}"
    );
    assert!(
        rows.iter()
            .any(|a| a["tool"] == "oauth.authorize" && a["ok"] == true),
        "{audit}"
    );

    // code reuse: invalid_grant and the grant made from it is revoked
    let r = exchange(&app, &client, redirect, &code, &p.verifier).await;
    assert_eq!(r.json()["error"], "invalid_grant");
    assert_eq!(
        mcp(&app, Some(&access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &client),
        ],
    )
    .await;
    assert_eq!(r.json()["error"], "invalid_grant");

    // a fresh connection: refresh rotates, reuse of the old refresh token revokes everything
    let (client, t1) = connect(&app, &["read", "write"]).await;
    assert_eq!(t1.scope, "read write");
    let names = tool_names(&app, &t1.access).await;
    assert!(names.contains(&"rate_book".to_string()));
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &t1.refresh),
            ("client_id", "flc_other"),
        ],
    )
    .await;
    assert_eq!(
        r.json()["error"],
        "invalid_grant",
        "another client's refresh"
    );
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &t1.refresh),
            ("client_id", &client),
            ("resource", RESOURCE),
        ],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let t2 = r.json();
    let (a2, r2) = (
        t2["access_token"].as_str().unwrap().to_string(),
        t2["refresh_token"].as_str().unwrap().to_string(),
    );
    assert_ne!(r2, t1.refresh);
    assert_eq!(t2["scope"], "read write");
    assert_eq!(tool_names(&app, &a2).await.len(), names.len());
    // narrower access token on request
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &r2),
            ("client_id", &client),
            ("scope", "read"),
        ],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let t3 = r.json();
    assert_eq!(t3["scope"], "read");
    let (a3, r3) = (
        t3["access_token"].as_str().unwrap().to_string(),
        t3["refresh_token"].as_str().unwrap().to_string(),
    );
    assert!(
        !tool_names(&app, &a3)
            .await
            .contains(&"rate_book".to_string())
    );
    // wider than granted
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &r3),
            ("client_id", &client),
            ("scope", "read send"),
        ],
    )
    .await;
    assert_eq!(r.json()["error"], "invalid_scope");
    // reuse of a rotated refresh token
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &t1.refresh),
            ("client_id", &client),
        ],
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"], "invalid_grant");
    for tok in [&a2, &a3] {
        assert_eq!(
            mcp(&app, Some(tok), "tools/list", json!({})).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &r3),
            ("client_id", &client),
        ],
    )
    .await;
    assert_eq!(
        r.json()["error"],
        "invalid_grant",
        "the whole family is gone"
    );
    let audit = app.get("/api/v1/me/tokens/audit").await.json();
    assert!(
        audit
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["tool"] == "oauth.refresh_reuse"),
        "{audit}"
    );

    // RFC 7009 revocation of a refresh token ends the grant
    let (client, t) = connect(&app, &["read"]).await;
    let r = post_form(
        &app,
        "/oauth/revoke",
        &[
            ("token", &t.refresh),
            ("token_type_hint", "refresh_token"),
            ("client_id", &client),
        ],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // unknown tokens: still 200
    assert_eq!(
        post_form(&app, "/oauth/revoke", &[("token", "whatever")])
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        post_form(&app, "/oauth/revoke", &[]).await.status,
        StatusCode::BAD_REQUEST
    );
    // revoking an access token of another client does nothing
    let (_, t) = connect(&app, &["read"]).await;
    post_form(
        &app,
        "/oauth/revoke",
        &[("token", &t.access), ("client_id", "flc_someone_else")],
    )
    .await;
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::OK
    );
    post_form(&app, "/oauth/revoke", &[("token", &t.access)]).await;
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // the refresh token still works after an access token was revoked
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &t.refresh),
            ("client_id", &client_of(&app, &t.refresh)),
        ],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
}

fn client_of(app: &TestApp, refresh: &str) -> String {
    let h = hex::encode(Sha256::digest(refresh.as_bytes()));
    let c = app.state.db.lock();
    c.query_row(
        "SELECT g.client_id FROM oauth_token t JOIN oauth_grant g ON g.id=t.grant_id WHERE t.token_hash=?1",
        [h],
        |r| r.get(0),
    )
    .unwrap()
}

#[tokio::test]
async fn authorized_apps_list_and_revoke() {
    let mut app = app().await;
    let (_, t) = connect(&app, &["read", "send"]).await;
    let apps = app.get("/api/v1/me/oauth/apps").await.json();
    let a = &apps[0];
    assert_eq!(a["clientName"], "Test client");
    assert_eq!(a["clientKind"], "dcr");
    assert_eq!(a["redirectHost"], "127.0.0.1");
    assert_eq!(a["scopes"], json!(["read", "send"]));
    assert!(a["lastUsedAt"].is_null());
    tool_names(&app, &t.access).await;
    let apps = app.get("/api/v1/me/oauth/apps").await.json();
    assert!(apps[0]["lastUsedAt"].is_string(), "{apps}");
    // another user cannot see or revoke it
    app.post(
        "/api/v1/users",
        &json!({"username": "bob", "password": "bob-pass", "role": "reader"}),
    )
    .await;
    let id = apps[0]["id"].as_i64().unwrap();
    let admin_cookie = app.cookie.clone();
    app.login("bob", "bob-pass").await;
    assert_eq!(app.get("/api/v1/me/oauth/apps").await.json(), json!([]));
    assert_eq!(
        app.delete(&format!("/api/v1/me/oauth/apps/{id}"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    app.cookie = admin_cookie;
    assert_eq!(
        app.delete(&format!("/api/v1/me/oauth/apps/{id}"))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(app.get("/api/v1/me/oauth/apps").await.json(), json!([]));
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let audit = app.get("/api/v1/me/tokens/audit").await.json();
    assert!(
        audit
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["tool"] == "oauth.revoke"),
        "{audit}"
    );

    // deny: the client gets access_denied, no grant
    let client = register_ok(&app).await;
    let p = pkce();
    let loc = authorize(&app, &authorize_url(&client, LOOPBACK, &p, &[])).await;
    let id = query_of(&loc)["request"].clone();
    let v = app
        .get(&format!("/api/v1/oauth/requests/{id}"))
        .await
        .json();
    assert_eq!(
        v["scopes"],
        json!(["read", "write", "send"]),
        "no scope: all"
    );
    let r = app
        .post(
            &format!("/api/v1/oauth/requests/{id}"),
            &json!({"approve": false, "csrf": v["csrf"]}),
        )
        .await;
    let q = query_of(r.json()["redirect"].as_str().unwrap());
    assert_eq!(q["error"], "access_denied");
    assert_eq!(q["state"], "st-123");
    assert!(!q.contains_key("code"));
    assert_eq!(app.get("/api/v1/me/oauth/apps").await.json(), json!([]));
}

#[tokio::test]
async fn client_id_metadata_documents() {
    let app = app().await;
    let p = pkce();
    let back = consent(
        &app,
        &authorize_url(CIMD_ID, CLAUDE_CB, &p, &[("scope", "read write send")]),
        &["read", "write", "send"],
    )
    .await;
    assert!(back.starts_with(&format!("{CLAUDE_CB}?code=")), "{back}");
    let q = query_of(&back);
    let r = exchange(&app, CIMD_ID, CLAUDE_CB, &q["code"], &p.verifier).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let apps = app.get("/api/v1/me/oauth/apps").await.json();
    assert_eq!(apps[0]["clientKind"], "cimd");
    assert_eq!(apps[0]["verifiedHost"], "claude.ai");
    assert_eq!(apps[0]["redirectHost"], "claude.ai");

    // consent page data of a CIMD client
    let loc = authorize(&app, &authorize_url(CIMD_ID, CLAUDE_CB, &pkce(), &[])).await;
    let id = query_of(&loc)["request"].clone();
    let v = app
        .get(&format!("/api/v1/oauth/requests/{id}"))
        .await
        .json();
    assert_eq!(v["client"]["verifiedHost"], "claude.ai");
    assert_eq!(v["loopback"], false);

    // untrusted host, document for another id, unknown document, redirect not in the document
    for (id, redirect) in [
        (
            "https://evil.example/client.json",
            "https://evil.example/cb",
        ),
        ("https://claude.ai/oauth/wrong-id", CLAUDE_CB),
        ("http://claude.ai/oauth/test-client-metadata", CLAUDE_CB),
        ("https://claude.ai/", CLAUDE_CB),
    ] {
        let loc = authorize(&app, &authorize_url(id, redirect, &pkce(), &[])).await;
        assert_eq!(loc, "/oauth/consent?error=invalid_client", "{id}");
    }
    let loc = authorize(
        &app,
        &authorize_url(CIMD_ID, "https://claude.ai/other_callback", &pkce(), &[]),
    )
    .await;
    assert_eq!(loc, "/oauth/consent?error=invalid_redirect_uri");
}

#[tokio::test]
async fn redirect_uri_validation_and_attacks() {
    let app = app().await;
    // registration: only trusted https hosts and loopback
    for bad in [
        vec!["https://evil.example/cb"],
        vec!["http://books.example.org/cb"],
        vec!["https://claude.ai/cb#x"],
        vec!["javascript:alert(1)"],
        vec![LOOPBACK, "https://claude.ai.evil.example/cb"],
        vec![],
    ] {
        let r = register(&app, &bad).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad:?}");
        assert_eq!(r.json()["error"], "invalid_redirect_uri");
    }
    let r = anon(
        &app,
        Method::POST,
        "/oauth/register",
        Some(&json!({"redirect_uris": [LOOPBACK], "grant_types": ["client_credentials"]})),
    )
    .await;
    assert_eq!(r.json()["error"], "invalid_client_metadata");
    let r = register(&app, &[CLAUDE_CB]).await;
    assert_eq!(r.status, StatusCode::CREATED);
    let claude_client = r.json()["client_id"].as_str().unwrap().to_string();
    let client = register_ok(&app).await;

    // wrong or foreign redirect URIs are never redirected to
    for redirect in [
        "https://evil.example/callback",
        "http://127.0.0.1:5555/callback/../evil",
        "http://127.0.0.1:5555/callbackx",
        "http://127.0.0.1:5555/callback?x=1",
        "http://localhost:5555/callback",
        "https://127.0.0.1/callback",
        "http://127.0.0.1:5555/callback#frag",
    ] {
        let loc = authorize(&app, &authorize_url(&client, redirect, &pkce(), &[])).await;
        assert_eq!(
            loc, "/oauth/consent?error=invalid_redirect_uri",
            "{redirect}"
        );
    }
    let loc = authorize(
        &app,
        &authorize_url(
            &claude_client,
            "https://claude.ai/api/mcp/auth_callback/",
            &pkce(),
            &[],
        ),
    )
    .await;
    assert_eq!(loc, "/oauth/consent?error=invalid_redirect_uri");
    // unknown client, missing redirect
    assert_eq!(
        authorize(&app, &authorize_url("flc_nope", LOOPBACK, &pkce(), &[])).await,
        "/oauth/consent?error=invalid_client"
    );
    assert_eq!(
        authorize(
            &app,
            &format!("/oauth/authorize?client_id={client}&response_type=code")
        )
        .await,
        "/oauth/consent?error=invalid_redirect_uri"
    );
    // repeated parameters
    let dup = format!(
        "{}&client_id=flc_other",
        authorize_url(&client, LOOPBACK, &pkce(), &[])
    );
    assert_eq!(
        authorize(&app, &dup).await,
        "/oauth/consent?error=invalid_request"
    );

    // valid client + redirect: errors go back to the client with state and iss
    let p = pkce();
    let cases: Vec<(String, &str)> = vec![
        (
            authorize_url(&client, LOOPBACK, &p, &[])
                .replace("code_challenge_method=S256", "code_challenge_method=plain"),
            "invalid_request",
        ),
        (
            authorize_url(&client, LOOPBACK, &p, &[]).replace(
                &format!("code_challenge={}", p.challenge),
                "code_challenge=",
            ),
            "invalid_request",
        ),
        (
            authorize_url(&client, LOOPBACK, &p, &[])
                .replace("response_type=code", "response_type=token"),
            "unsupported_response_type",
        ),
        (
            authorize_url(&client, LOOPBACK, &p, &[("scope", "read admin")]),
            "invalid_scope",
        ),
        (
            authorize_url(&client, LOOPBACK, &p, &[]).replace(
                "resource=https%3A%2F%2Fbooks.example.org%2Fmcp",
                "resource=https%3A%2F%2Fother.example%2Fmcp",
            ),
            "invalid_target",
        ),
    ];
    for (url, err) in cases {
        let loc = authorize(&app, &url).await;
        assert!(loc.starts_with(&format!("{LOOPBACK}?")), "{loc}");
        let q = query_of(&loc);
        assert_eq!(q["error"], err, "{url}");
        assert_eq!(q["state"], "st-123");
        assert_eq!(q["iss"], PUBLIC);
    }
    // a code bound to one redirect URI cannot be exchanged with another
    let back = consent(
        &app,
        &authorize_url(&client, "http://127.0.0.1:1111/callback", &p, &[]),
        &["read"],
    )
    .await;
    let code = query_of(&back)["code"].clone();
    let r = exchange(
        &app,
        &client,
        "http://127.0.0.1:2222/callback",
        &code,
        &p.verifier,
    )
    .await;
    assert_eq!(r.json()["error"], "invalid_grant");
    let r = exchange(
        &app,
        &claude_client,
        "http://127.0.0.1:1111/callback",
        &code,
        &p.verifier,
    )
    .await;
    assert_eq!(r.json()["error"], "invalid_grant", "another client");
    // wrong resource at the token endpoint
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", "http://127.0.0.1:1111/callback"),
            ("client_id", &client),
            ("code_verifier", &p.verifier),
            ("resource", "https://other.example/mcp"),
        ],
    )
    .await;
    assert_eq!(r.json()["error"], "invalid_target");
    // token endpoint input checks
    let r = anon(
        &app,
        Method::POST,
        "/oauth/token",
        Some(&json!({"grant_type": "authorization_code"})),
    )
    .await;
    assert_eq!(r.json()["error"], "invalid_request", "JSON body");
    let r = post_form(
        &app,
        "/oauth/token",
        &[("grant_type", "client_credentials"), ("client_id", &client)],
    )
    .await;
    assert_eq!(r.json()["error"], "unsupported_grant_type");
    let r = post_form(
        &app,
        "/oauth/token",
        &[("grant_type", "authorization_code"), ("code", &code)],
    )
    .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.json()["error"], "invalid_client");
}

#[tokio::test]
async fn audience_binding_and_expiry() {
    let app = app().await;
    let (_, t) = connect(&app, &["read"]).await;
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::OK
    );
    // a token bound to another resource is refused
    app.state
        .db
        .lock()
        .execute(
            "UPDATE oauth_grant SET resource='https://other.example/mcp'",
            [],
        )
        .unwrap();
    app.state.tokens.invalidate();
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    app.state
        .db
        .lock()
        .execute("UPDATE oauth_grant SET resource=?1", [RESOURCE])
        .unwrap();
    app.state.tokens.invalidate();
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::OK
    );
    // expired access token (also when cached)
    app.state
        .db
        .lock()
        .execute(
            "UPDATE oauth_token SET expires_at=1 WHERE kind='access'",
            [],
        )
        .unwrap();
    app.state.tokens.invalidate();
    let r = mcp(&app, Some(&t.access), "tools/list", json!({})).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.header("www-authenticate").contains("invalid_token"));
    // expired refresh token
    app.state
        .db
        .lock()
        .execute(
            "UPDATE oauth_token SET expires_at=1 WHERE kind='refresh'",
            [],
        )
        .unwrap();
    let r = post_form(
        &app,
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &t.refresh),
            ("client_id", &client_of(&app, &t.refresh)),
        ],
    )
    .await;
    assert_eq!(r.json()["error"], "invalid_grant");
    // cleanup removes expired tokens and the empty grant
    freelib_server::oauth::cleanup(&app.state).await;
    assert_eq!(app.get("/api/v1/me/oauth/apps").await.json(), json!([]));
    // a personal API token still works next to OAuth
    let r = app
        .post(
            "/api/v1/me/tokens",
            &json!({"name": "cli", "scopes": ["read"]}),
        )
        .await;
    let secret = r.json()["secret"].as_str().unwrap().to_string();
    assert_eq!(
        mcp(&app, Some(&secret), "tools/list", json!({}))
            .await
            .status,
        StatusCode::OK
    );
    // the MCP switch applies to OAuth tokens too
    let (_, t) = connect(&app, &["read"]).await;
    app.put("/api/v1/settings", &json!({"mcp": {"enabled": false}}))
        .await;
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn registration_limits_and_cleanup() {
    let app = app().await;
    for i in 0..20 {
        assert_eq!(
            register(&app, &[LOOPBACK]).await.status,
            StatusCode::CREATED,
            "{i}"
        );
    }
    let r = register(&app, &[LOOPBACK]).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);

    // the cap on registered clients (recent, unused clients are kept)
    let app = self::app().await;
    {
        let c = app.state.db.lock();
        let now = freelib_server::util::unix_now();
        for i in 0..freelib_server::oauth::MAX_CLIENTS {
            c.execute(
                "INSERT INTO oauth_client(client_id, name, redirect_uris, created_at) VALUES (?1, 'x', '[]', ?2)",
                rusqlite::params![format!("flc_fill{i}"), now],
            )
            .unwrap();
        }
    }
    assert_eq!(
        register(&app, &[LOOPBACK]).await.status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    // old unused clients make room
    app.state.db.lock().execute("UPDATE oauth_client SET created_at = created_at - 7200 WHERE client_id LIKE 'flc_fill1%'", []).unwrap();
    assert_eq!(
        register(&app, &[LOOPBACK]).await.status,
        StatusCode::CREATED
    );
    // the periodic cleanup drops clients never used for a day, keeps used ones
    let (client, _) = connect(&app, &["read"]).await;
    app.state
        .db
        .lock()
        .execute(
            "UPDATE oauth_client SET created_at = created_at - 200000",
            [],
        )
        .unwrap();
    freelib_server::oauth::cleanup(&app.state).await;
    let left: Vec<String> = {
        let c = app.state.db.lock();
        let mut st = c.prepare("SELECT client_id FROM oauth_client").unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(left, vec![client]);
}

#[tokio::test]
async fn open_mode_grants_do_not_survive_the_first_account() {
    let mut app = TestApp::new(|cfg, _| {
        cfg.public_url = Some(PUBLIC.into());
        cfg.allowed_hosts = vec!["books.example.org".into()];
    })
    .await;
    let (_, t) = connect(&app, &["read"]).await;
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::OK
    );
    app.post(
        "/api/v1/users",
        &json!({"username": "admin", "password": "pw-admin", "role": "admin"}),
    )
    .await;
    app.login("admin", "pw-admin").await;
    assert_eq!(
        mcp(&app, Some(&t.access), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(app.get("/api/v1/me/oauth/apps").await.json(), json!([]));
}

// ---------------------------------------------------------------- secrets at rest

fn raw_smtp(app: &TestApp) -> String {
    freelib_server::db::get_setting_raw(&app.state.db.lock(), "smtp")
        .unwrap()
        .unwrap_or_default()
}

#[tokio::test]
async fn smtp_password_encrypted_at_rest() {
    let app = app().await;
    let r = app
        .put(
            "/api/v1/settings",
            &json!({"smtp": {"host": "smtp.example.org", "password": "s3cret-Pass!"}}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["smtp"]["passwordSet"], true);
    assert!(!r.text().contains("s3cret"));
    let raw = raw_smtp(&app);
    assert!(!raw.contains("s3cret-Pass!"), "{raw}");
    assert!(raw.contains("\"password\":\"enc:v1:"), "{raw}");
    // the file with the key is private
    use std::os::unix::fs::PermissionsExt;
    let key = app.root().join("data/secret.key");
    assert_eq!(
        std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // decrypted for sending
    let smtp: freelib_server::db::SmtpConfig =
        freelib_server::db::get_setting(&app.state.db.lock(), "smtp").unwrap();
    assert_eq!(
        smtp.revealed(&app.state.secrets)
            .unwrap()
            .password
            .as_deref(),
        Some("s3cret-Pass!")
    );
    // other settings changes keep it; an empty string clears it
    app.put("/api/v1/settings", &json!({"smtp": {"port": 465}}))
        .await;
    assert!(raw_smtp(&app).contains("enc:v1:"));
    app.put("/api/v1/settings", &json!({"smtp": {"password": ""}}))
        .await;
    assert!(!raw_smtp(&app).contains("enc:v1:"));
}

#[tokio::test]
async fn plain_secrets_migrated_at_start_and_wrong_key_refused() {
    let app = app().await;
    freelib_server::db::put_setting_raw(
        &app.state.db.lock(),
        "smtp",
        r#"{"host":"smtp.example.org","password":"legacy-plain"}"#,
    )
    .unwrap();
    let app = app.restart().await;
    let raw = raw_smtp(&app);
    assert!(
        !raw.contains("legacy-plain") && raw.contains("enc:v1:"),
        "{raw}"
    );
    let first = raw.clone();
    // idempotent
    let app = app.restart().await;
    assert_eq!(raw_smtp(&app), first);
    let root = app.root().to_path_buf();
    let old_key = std::fs::read_to_string(root.join("data/secret.key")).unwrap();

    // a different key: refuse to start, clearly
    let mut cfg = freelib_server::Config::for_dir(&root);
    cfg.secret_key = Some(freelib_server::secrets::generate_key_text().into());
    let err = match freelib_server::init(cfg.clone()).await {
        Ok(_) => panic!("started with the wrong key"),
        Err(e) => format!("{e:#}"),
    };
    assert!(
        err.contains("FREELIB_SECRET_KEY_OLD") && err.contains("forget-secrets"),
        "{err}"
    );
    assert!(!err.contains("legacy-plain"));
    // rotation to the new key
    cfg.secret_key_old = Some(old_key.trim().into());
    let st = freelib_server::init(cfg.clone()).await.unwrap();
    let raw = freelib_server::db::get_setting_raw(&st.db.lock(), "smtp")
        .unwrap()
        .unwrap();
    assert_ne!(raw, first);
    let smtp: freelib_server::db::SmtpConfig = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        smtp.revealed(&st.secrets).unwrap().password.as_deref(),
        Some("legacy-plain")
    );
    drop(st);
    // the new key alone now works; the old one alone no longer does
    cfg.secret_key_old = None;
    freelib_server::init(cfg.clone()).await.unwrap();
    let mut cfg_old = cfg.clone();
    cfg_old.secret_key = Some(old_key.trim().into());
    assert!(freelib_server::init(cfg_old).await.is_err());
    drop(app);
}
