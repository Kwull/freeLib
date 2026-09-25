//! MCP endpoint and API tokens: token lifecycle, scopes, the protocol round trip over the
//! streamable HTTP transport, every tool, prompts, audit log, rate limiting.

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{Resp, TestApp, fake_openlibrary, make_library};
use serde_json::{Value, json};

const ADMIN_PW: &str = "admin-pass";

async fn app_with_library(customize: impl FnOnce(&mut freelib_server::Config)) -> (TestApp, i64) {
    let mut app = TestApp::new(|cfg, root| {
        make_library(root, 300);
        cfg.admin_password = Some(ADMIN_PW.into());
        customize(cfg);
    })
    .await;
    assert_eq!(app.login("admin", ADMIN_PW).await.status, StatusCode::OK);
    let lib = app.import_library().await;
    (app, lib["id"].as_i64().unwrap())
}

async fn new_token(app: &TestApp, name: &str, scopes: &[&str]) -> (String, Value) {
    let r = app
        .post(
            "/api/v1/me/tokens",
            &json!({"name": name, "scopes": scopes}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let v = r.json();
    (
        v["secret"].as_str().unwrap().to_string(),
        v["token"].clone(),
    )
}

/// One JSON-RPC request to `/mcp`; returns the HTTP response.
async fn rpc_raw(app: &TestApp, token: Option<&str>, method: &str, params: Value) -> Resp {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let mut b = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, "localhost:8080")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-06-18");
    if let Some(t) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    app.send(b.body(Body::from(body.to_string())).unwrap())
        .await
}

/// The JSON-RPC message of a response (plain JSON or the last SSE `data:` line).
fn message(r: &Resp) -> Value {
    let ct = r.header("content-type");
    if ct.contains("text/event-stream") {
        let text = r.text();
        let data = text
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(str::trim)
            .rfind(|d| !d.is_empty())
            .expect("an SSE data line");
        serde_json::from_str(data).unwrap()
    } else {
        r.json()
    }
}

async fn rpc(app: &TestApp, token: &str, method: &str, params: Value) -> Value {
    let r = rpc_raw(app, Some(token), method, params).await;
    assert_eq!(r.status, StatusCode::OK, "{method}: {}", r.text());
    let m = message(&r);
    assert!(m.get("error").is_none(), "{method}: {m}");
    m["result"].clone()
}

/// Calls a tool; (isError, structured result or error text).
async fn tool(app: &TestApp, token: &str, name: &str, args: Value) -> (bool, Value) {
    let res = rpc(
        app,
        token,
        "tools/call",
        json!({"name": name, "arguments": args}),
    )
    .await;
    let is_error = res["isError"].as_bool().unwrap_or(false);
    if is_error {
        (true, res["content"][0]["text"].clone())
    } else {
        (false, res["structuredContent"].clone())
    }
}

async fn ok_tool(app: &TestApp, token: &str, name: &str, args: Value) -> Value {
    let (err, v) = tool(app, token, name, args.clone()).await;
    assert!(!err, "{name}({args}) failed: {v}");
    v
}

#[tokio::test]
async fn token_lifecycle() {
    let (mut app, _lib) = app_with_library(|_| {}).await;
    // created: the secret once, with the recognisable prefix
    let (secret, tok) = new_token(&app, "Claude Desktop", &["send", "read", "read"]).await;
    assert!(secret.starts_with("fl_") && secret.len() == 46, "{secret}");
    assert_eq!(tok["scopes"], json!(["read", "send"]));
    assert_eq!(tok["prefix"], &secret[..11]);
    assert!(tok["lastUsedAt"].is_null());
    // listed without the secret
    let list = app.get("/api/v1/me/tokens").await;
    assert!(!list.text().contains(&secret));
    let lv = list.json();
    assert_eq!(lv["tokens"][0]["name"], "Claude Desktop");
    assert_eq!(lv["scopes"], json!(["read", "write", "send"]));
    assert_eq!(lv["mcp"]["enabled"], true);
    assert!(lv["mcp"]["url"].as_str().unwrap().ends_with("/mcp"));
    // only the SHA-256 of the secret is stored
    {
        let c = app.state.db.lock();
        let (hash, prefix): (String, String) = c
            .query_row("SELECT token_hash, prefix FROM api_token", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(hash, freelib_server::tokens::hash(&secret));
        assert_eq!(hash.len(), 64);
        assert!(!hash.contains(&secret[3..]) && prefix.len() == 11);
    }
    // validation
    for bad in [
        json!({"name": "", "scopes": ["read"]}),
        json!({"name": "x", "scopes": []}),
        json!({"name": "x", "scopes": ["admin"]}),
        json!({"name": "x", "scopes": ["read"], "expiresInDays": 99999}),
    ] {
        assert_eq!(
            app.post("/api/v1/me/tokens", &bad).await.status,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    // the token works for MCP; its use is recorded
    let init = rpc(
        &app,
        &secret,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
    )
    .await;
    assert_eq!(init["serverInfo"]["name"], "freelib");
    assert!(
        init["instructions"]
            .as_str()
            .unwrap()
            .contains("suggest_candidates")
    );
    assert!(init["capabilities"]["tools"].is_object());
    assert!(init["capabilities"]["prompts"].is_object());
    let lv = app.get("/api/v1/me/tokens").await.json();
    assert!(lv["tokens"][0]["lastUsedAt"].is_string());
    // no / bad token: 401 with a Bearer challenge
    let r = rpc_raw(&app, None, "tools/list", json!({})).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.header("www-authenticate").starts_with("Bearer"));
    let forged = format!("fl_{}", "A".repeat(43));
    assert_eq!(
        rpc_raw(&app, Some(&forged), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // a session cookie is no MCP credential
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .header(header::HOST, "localhost:8080")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(c) = &app.cookie {
        req = req.header(header::COOKIE, c);
    }
    let r = app
        .send(
            req.body(Body::from(
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            ))
            .unwrap(),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // another user cannot revoke it; tokens are per user
    app.post(
        "/api/v1/users",
        &json!({"username": "bob", "password": "bob-pass", "role": "reader"}),
    )
    .await;
    let admin_cookie = app.cookie.clone();
    app.login("bob", "bob-pass").await;
    let id = tok["id"].as_i64().unwrap();
    assert_eq!(
        app.delete(&format!("/api/v1/me/tokens/{id}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get("/api/v1/me/tokens").await.json()["tokens"],
        json!([])
    );
    app.cookie = admin_cookie;
    // revoked: refused at once (the cache is cleared)
    assert_eq!(
        app.delete(&format!("/api/v1/me/tokens/{id}")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        rpc_raw(&app, Some(&secret), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // expiry
    let r = app
        .post(
            "/api/v1/me/tokens",
            &json!({"name": "short", "scopes": ["read"], "expiresInDays": 1}),
        )
        .await
        .json();
    assert!(r["token"]["expiresAt"].is_string());
    let short = r["secret"].as_str().unwrap().to_string();
    {
        let c = app.state.db.lock();
        c.execute(
            "UPDATE api_token SET expires_at='2000-01-01T00:00:00Z' WHERE name='short'",
            [],
        )
        .unwrap();
    }
    assert_eq!(
        rpc_raw(&app, Some(&short), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // deleting the user removes their tokens
    let (bob_secret, _) = {
        let admin = app.cookie.clone();
        app.login("bob", "bob-pass").await;
        let t = new_token(&app, "bob", &["read"]).await;
        app.cookie = admin;
        t
    };
    assert_eq!(
        rpc_raw(&app, Some(&bob_secret), "tools/list", json!({}))
            .await
            .status,
        StatusCode::OK
    );
    let users = app.get("/api/v1/users").await.json();
    let bob = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "bob")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    app.delete(&format!("/api/v1/users/{bob}")).await;
    assert_eq!(
        rpc_raw(&app, Some(&bob_secret), "tools/list", json!({}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let n: i64 = app
        .state
        .db
        .lock()
        .query_row(
            "SELECT count(*) FROM api_token WHERE user_id=?1",
            [bob],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn admin_switch_and_rate_limit() {
    let (app, _lib) = app_with_library(|cfg| cfg.mcp_rate_per_min = 5).await;
    let (secret, _) = new_token(&app, "t", &["read"]).await;
    // the admin switch
    let s = app.get("/api/v1/settings").await.json();
    assert_eq!(s["mcp"]["enabled"], true);
    let r = app
        .put("/api/v1/settings", &json!({"mcp": {"enabled": false}}))
        .await;
    assert_eq!(r.json()["mcp"]["enabled"], false);
    let r = rpc_raw(&app, Some(&secret), "tools/list", json!({})).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(r.text().contains("disabled"));
    assert_eq!(
        app.get("/api/v1/me/tokens").await.json()["mcp"]["enabled"],
        false
    );
    app.put("/api/v1/settings", &json!({"mcp": {"enabled": true}}))
        .await;
    // 5 requests per minute and token
    for _ in 0..5 {
        assert_eq!(
            rpc_raw(&app, Some(&secret), "tools/list", json!({}))
                .await
                .status,
            StatusCode::OK
        );
    }
    let r = rpc_raw(&app, Some(&secret), "tools/list", json!({})).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(!r.header("retry-after").is_empty());
    // another token has its own budget
    let (other, _) = new_token(&app, "u", &["read"]).await;
    assert_eq!(
        rpc_raw(&app, Some(&other), "tools/list", json!({}))
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn scopes_are_enforced_per_tool() {
    let (app, lib) = app_with_library(|_| {}).await;
    let (read, _) = new_token(&app, "read", &["read"]).await;
    let (write, _) = new_token(&app, "write", &["write"]).await;
    let (send, _) = new_token(&app, "send", &["send"]).await;
    let names = |v: &Value| -> Vec<String> {
        v["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    };
    let tr = names(&rpc(&app, &read, "tools/list", json!({})).await);
    let tw = names(&rpc(&app, &write, "tools/list", json!({})).await);
    let ts = names(&rpc(&app, &send, "tools/list", json!({})).await);
    assert!(tr.contains(&"search_books".to_string()) && tr.contains(&"list_devices".to_string()));
    assert!(
        !tr.iter()
            .any(|t| ["rate_book", "add_to_shelf", "send_books", "get_job"].contains(&t.as_str()))
    );
    assert_eq!(tw, ["add_to_shelf", "remove_from_shelf", "rate_book"]);
    assert_eq!(ts, ["send_books", "get_job"]);
    assert_eq!(tr.len() + tw.len() + ts.len(), 17);

    let book = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=1"
        ))
        .await
        .json()["books"][0]["id"]
        .as_i64()
        .unwrap();
    // every write / send tool is refused with a read token, and vice versa
    for (name, args) in [
        ("rate_book", json!({"id": book, "rating": 4})),
        (
            "add_to_shelf",
            json!({"shelf": "x", "create": true, "book_ids": [book]}),
        ),
        (
            "remove_from_shelf",
            json!({"shelf": "x", "book_ids": [book]}),
        ),
        (
            "send_books",
            json!({"book_ids": [book], "device": "default"}),
        ),
        ("get_job", json!({"id": "nope"})),
    ] {
        let (err, msg) = tool(&app, &read, name, args).await;
        assert!(err, "{name}");
        assert!(msg.as_str().unwrap().contains("scope"), "{name}: {msg}");
    }
    for name in [
        "search_books",
        "get_book",
        "get_reading_profile",
        "list_shelves",
    ] {
        let (err, msg) = tool(&app, &write, name, json!({"id": book, "query": "ab"})).await;
        assert!(
            err && msg.as_str().unwrap().contains("'read' scope"),
            "{name}: {msg}"
        );
    }
    let (err, _) = tool(&app, &write, "rate_book", json!({"id": book, "rating": 4})).await;
    assert!(!err);
    let (err, msg) = tool(&app, &send, "rate_book", json!({"id": book, "rating": 4})).await;
    assert!(err, "{msg}");
    // unknown tools
    let (err, msg) = tool(&app, &read, "drop_tables", json!({})).await;
    assert!(err && msg.as_str().unwrap().contains("unknown tool"));
    // the audit log records calls, including refusals, newest first
    let audit = app.get("/api/v1/me/tokens/audit").await.json();
    let rows = audit.as_array().unwrap();
    assert!(rows.len() >= 12);
    assert_eq!(rows[0]["tool"], "drop_tables");
    assert_eq!(rows[0]["ok"], false);
    assert_eq!(rows[0]["tokenName"], "read");
    assert!(
        rows.iter()
            .any(|r| r["tool"] == "rate_book" && r["ok"] == true && r["tokenName"] == "write")
    );
    assert!(rows.iter().all(|r| r["at"].is_string()));
}

#[tokio::test]
async fn protocol_round_trip_and_every_tool() {
    let ol = fake_openlibrary().await;
    let url = ol.url.clone();
    let (app, lib) = app_with_library(move |cfg| cfg.openlibrary_url = url).await;
    let (tok, _) = new_token(&app, "all", &["read", "write", "send"]).await;

    // initialize (stateless: no session id needed afterwards), tools/list, prompts
    let init = rpc(
        &app,
        &tok,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}),
    )
    .await;
    assert!(init["protocolVersion"].is_string());
    let r = rpc_raw(&app, Some(&tok), "notifications/initialized", Value::Null).await;
    assert!(r.status.is_success(), "{}", r.status);
    let tools = rpc(&app, &tok, "tools/list", json!({})).await;
    let tools = tools["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 17);
    for t in tools {
        assert!(t["description"].as_str().unwrap().len() > 20, "{t}");
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(t["annotations"]["readOnlyHint"].is_boolean(), "{t}");
    }
    let search = tools.iter().find(|t| t["name"] == "search_books").unwrap();
    assert!(search["inputSchema"]["properties"]["kids_max_age"].is_object());
    let prompts = rpc(&app, &tok, "prompts/list", json!({})).await;
    let pn: Vec<&str> = prompts["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(pn, ["suggest_next_book", "books_for_kid", "similar_to"]);
    let p = rpc(
        &app,
        &tok,
        "prompts/get",
        json!({"name": "books_for_kid", "arguments": {"age": "7"}}),
    )
    .await;
    let text = p["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("kids_max_age=7"), "{text}");
    let p = rpc(
        &app,
        &tok,
        "prompts/get",
        json!({"name": "similar_to", "arguments": {"book_id": "12"}}),
    )
    .await;
    assert!(
        p["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("seed_book_ids=[12]")
    );
    let p = rpc(
        &app,
        &tok,
        "prompts/get",
        json!({"name": "suggest_next_book", "arguments": {}}),
    )
    .await;
    assert!(
        p["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("get_reading_profile")
    );
    let r = rpc_raw(
        &app,
        Some(&tok),
        "prompts/get",
        json!({"name": "books_for_kid", "arguments": {}}),
    )
    .await;
    assert!(message(&r)["error"].is_object(), "age is required");

    // list_libraries
    let libs = ok_tool(&app, &tok, "list_libraries", json!({})).await;
    assert_eq!(libs["libraries"][0]["id"], lib);
    assert_eq!(libs["libraries"][0]["default"], true);

    // pick test data through the REST API
    let all = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5000"
        ))
        .await
        .json();
    let books = all["books"].as_array().unwrap().clone();
    let with_series = books
        .iter()
        .find(|b| b["series"].is_object() && b["authors"].as_array().unwrap().len() == 1)
        .unwrap()
        .clone();
    let author_id = with_series["authors"][0]["id"].as_i64().unwrap();
    let series_id = with_series["series"]["id"].as_i64().unwrap();
    let book_id = with_series["id"].as_i64().unwrap();
    let word: String = with_series["title"]
        .as_str()
        .unwrap()
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| w.chars().count() >= 4)
        .unwrap()
        .to_string();

    // search_books: text, author, filters, sorting, pagination
    let r = ok_tool(
        &app,
        &tok,
        "search_books",
        json!({"query": word, "limit": 5}),
    )
    .await;
    assert!(r["total"].as_i64().unwrap() >= 1);
    assert!(r["books"].as_array().unwrap().len() <= 5);
    let b0 = &r["books"][0];
    for k in [
        "id",
        "title",
        "authors",
        "genres",
        "myRating",
        "libraryRating",
        "kidsAge",
        "openLibrary",
    ] {
        assert!(b0.get(k).is_some(), "{k} in {b0}");
    }
    let r = ok_tool(
        &app,
        &tok,
        "search_books",
        json!({"author_id": author_id, "limit": 2}),
    )
    .await;
    let total = r["total"].as_i64().unwrap();
    assert!(r["books"].as_array().unwrap().iter().all(|b| {
        b["authors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == author_id)
    }));
    if total > 2 {
        let next = r["nextCursor"].as_str().unwrap();
        let r2 = ok_tool(
            &app,
            &tok,
            "search_books",
            json!({"author_id": author_id, "limit": 2, "cursor": next}),
        )
        .await;
        assert_ne!(r2["books"][0]["id"], r["books"][0]["id"]);
    }
    let r = ok_tool(&app, &tok, "search_books", json!({"added_after": "1900-01-01", "min_library_rating": 4, "sort": "library_rating", "limit": 50})).await;
    let libr: Vec<i64> = r["books"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["libraryRating"].as_i64().unwrap())
        .collect();
    assert!(!libr.is_empty() && libr.iter().all(|x| *x >= 4));
    assert!(libr.windows(2).all(|w| w[0] >= w[1]), "{libr:?}");
    let r = ok_tool(
        &app,
        &tok,
        "search_books",
        json!({"genre": "Children's", "kids_max_age": 12, "limit": 50}),
    )
    .await;
    assert!(r["books"].as_array().unwrap().iter().all(|b| {
        let a = b["kidsAge"].as_str().unwrap();
        ["0+", "6+", "12+"].contains(&a)
    }));
    let (err, msg) = tool(&app, &tok, "search_books", json!({})).await;
    assert!(err && msg.as_str().unwrap().contains("give a query"));
    let r = ok_tool(
        &app,
        &tok,
        "search_books",
        json!({"author": with_series["authors"][0]["name"], "limit": 3}),
    )
    .await;
    assert!(r["matched"].as_str().unwrap().starts_with("author:"));

    // get_book, get_author, list_author_books, get_series, list_genres
    let b = ok_tool(&app, &tok, "get_book", json!({"id": book_id})).await;
    assert_eq!(b["id"], book_id);
    assert_eq!(b["library"], lib);
    assert!(b["annotation"].is_string() || b["annotation"].is_null());
    assert!(b["formats"].as_array().unwrap().iter().any(|f| f == "epub"));
    assert!(b["myHistory"]["lastSent"].is_null());
    assert!(b["openLibrary"].is_object());
    let (err, _) = tool(&app, &tok, "get_book", json!({"id": 999_999})).await;
    assert!(err);
    let a = ok_tool(&app, &tok, "get_author", json!({"id": author_id})).await;
    assert_eq!(a["id"], author_id);
    assert!(a["books"].as_i64().unwrap() >= 1);
    assert!(!a["bestRated"].as_array().unwrap().is_empty());
    let a2 = ok_tool(
        &app,
        &tok,
        "get_author",
        json!({"name": with_series["authors"][0]["name"]}),
    )
    .await;
    assert_eq!(a2["id"], author_id);
    let lb = ok_tool(
        &app,
        &tok,
        "list_author_books",
        json!({"author_id": author_id, "limit": 100}),
    )
    .await;
    assert_eq!(lb["total"].as_i64().unwrap(), a["books"].as_i64().unwrap());
    let s = ok_tool(&app, &tok, "get_series", json!({"id": series_id})).await;
    assert_eq!(s["id"], series_id);
    assert!(
        s["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == book_id)
    );
    assert!(s["nextUnread"].is_object());
    let g = ok_tool(&app, &tok, "list_genres", json!({})).await;
    let tops = g["genres"].as_array().unwrap();
    assert!(!tops.is_empty());
    assert!(tops.iter().all(|t| t["books"].as_i64().unwrap() > 0));

    // write tools: rate, shelves
    let r = ok_tool(&app, &tok, "rate_book", json!({"id": book_id, "rating": 5})).await;
    assert_eq!(r["myRating"], 5);
    let (err, _) = tool(&app, &tok, "rate_book", json!({"id": book_id, "rating": 7})).await;
    assert!(err);
    let others: Vec<i64> = books
        .iter()
        .filter(|b| b["authors"][0]["id"] != author_id)
        .take(2)
        .map(|b| b["id"].as_i64().unwrap())
        .collect();
    let (err, msg) = tool(
        &app,
        &tok,
        "add_to_shelf",
        json!({"shelf": "To read", "book_ids": others}),
    )
    .await;
    assert!(err && msg.as_str().unwrap().contains("create=true"));
    let sh = ok_tool(
        &app,
        &tok,
        "add_to_shelf",
        json!({"shelf": "To read", "create": true, "book_ids": others}),
    )
    .await;
    assert_eq!(sh["shelf"]["count"], 2);
    let shelf_id = sh["shelf"]["id"].as_i64().unwrap();
    let sh = ok_tool(
        &app,
        &tok,
        "remove_from_shelf",
        json!({"shelf_id": shelf_id, "book_ids": [others[0]]}),
    )
    .await;
    assert_eq!(sh["shelf"]["count"], 1);
    let ls = ok_tool(&app, &tok, "list_shelves", json!({})).await;
    assert_eq!(ls["shelves"][0]["name"], "To read");
    // the REST API sees the same data
    let d = app
        .get(&format!("/api/v1/libraries/{lib}/books/{book_id}"))
        .await
        .json();
    assert_eq!(d["rating"], 5);

    // external rating (the fake Open Library)
    let e = ok_tool(&app, &tok, "get_external_rating", json!({"id": book_id})).await;
    assert_eq!(e["source"], "openlibrary");
    assert_eq!(e["status"], "found", "{e}");
    assert!(
        e["url"]
            .as_str()
            .unwrap()
            .starts_with("https://openlibrary.org/works/OL")
    );
    assert!(e["votes"].as_u64().unwrap() >= 1);
    let hits = ol.count();
    let e2 = ok_tool(&app, &tok, "get_external_rating", json!({"id": book_id})).await;
    assert_eq!(e2["avg"], e["avg"]);
    assert_eq!(ol.count(), hits, "cached: no second lookup");
    let b = ok_tool(&app, &tok, "get_book", json!({"id": book_id})).await;
    assert_eq!(b["openLibrary"]["status"], "found");

    // devices and sending (a download device: no SMTP needed)
    let devs = ok_tool(&app, &tok, "list_devices", json!({})).await;
    let devs = devs["devices"].as_array().unwrap().clone();
    assert!(devs.iter().filter(|d| d["default"] == true).count() == 1);
    let dl = devs
        .iter()
        .find(|d| d["kind"] == "download" && d["format"] == "epub")
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let job = ok_tool(
        &app,
        &tok,
        "send_books",
        json!({"book_ids": [book_id], "device": dl}),
    )
    .await;
    let job_id = job["jobId"].as_str().unwrap().to_string();
    let done = app.wait_job(&job_id).await;
    assert_eq!(done["state"], "done", "{done}");
    let j = ok_tool(&app, &tok, "get_job", json!({"id": job_id})).await;
    assert_eq!(j["state"], "done");
    let (err, _) = tool(&app, &tok, "get_job", json!({"id": "nope"})).await;
    assert!(err);
    // sending respects the e-mail allowlist (the default Kindle device has no address: refused)
    let kindle = devs.iter().find(|d| d["kind"] == "email").unwrap()["id"]
        .as_i64()
        .unwrap();
    let (err, _) = tool(
        &app,
        &tok,
        "send_books",
        json!({"book_ids": [book_id], "device": kindle}),
    )
    .await;
    assert!(err);
    let b = ok_tool(&app, &tok, "get_book", json!({"id": book_id})).await;
    assert!(b["myHistory"]["lastDownloaded"].is_string(), "{b}");

    // reading profile and suggestions
    let p = ok_tool(&app, &tok, "get_reading_profile", json!({})).await;
    assert_eq!(p["counts"]["rated"], 1);
    assert_eq!(p["ratings"][0]["id"], book_id);
    assert_eq!(p["shelves"][0]["items"].as_array().unwrap().len(), 1);
    assert_eq!(p["recent"][0]["action"], "download");
    assert!(!p["topGenres"].as_array().unwrap().is_empty());
    assert_eq!(p["topAuthors"][0]["id"], author_id);
    let s = ok_tool(&app, &tok, "suggest_candidates", json!({"limit": 10})).await;
    let cands = s["candidates"].as_array().unwrap();
    assert!(!cands.is_empty(), "{s}");
    assert!(
        cands.iter().all(|c| c["id"] != book_id),
        "seeds and read books are excluded"
    );
    assert!(
        cands
            .iter()
            .all(|c| !c["reasons"].as_array().unwrap().is_empty())
    );
    let scores: Vec<f64> = cands.iter().map(|c| c["score"].as_f64().unwrap()).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]));
    let s = ok_tool(&app, &tok, "suggest_candidates", json!({"seed_book_ids": [book_id], "use_profile": false, "exclude_read": false, "limit": 5})).await;
    assert!(!s["candidates"].as_array().unwrap().is_empty());
    assert_eq!(s["seeds"][0]["id"], book_id);
}

#[tokio::test]
async fn device_order_is_per_user_and_picks_the_default() {
    let (mut app, _lib) = app_with_library(|_| {}).await;
    let devs = app.get("/api/v1/devices").await.json();
    let ids: Vec<i64> = devs
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_i64().unwrap())
        .collect();
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "unordered: oldest first"
    );
    // move the last device to the front and the first to second place
    let want = vec![ids[ids.len() - 1], ids[0]];
    let r = app
        .put("/api/v1/devices/order", &json!({"ids": want}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let got: Vec<i64> = r
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_i64().unwrap())
        .collect();
    assert_eq!(&got[..2], &want[..]);
    assert_eq!(got.len(), ids.len(), "the rest follow");
    let again: Vec<i64> = app
        .get("/api/v1/devices")
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_i64().unwrap())
        .collect();
    assert_eq!(again, got);
    // MCP: list_devices marks the first as default; 'default' sends to it
    let (tok, _) = new_token(&app, "all", &["read", "send"]).await;
    let l = ok_tool(&app, &tok, "list_devices", json!({})).await;
    assert_eq!(l["devices"][0]["id"], want[0]);
    assert_eq!(l["devices"][0]["default"], true);
    assert_eq!(l["devices"][1]["default"], false);
    // a new device is appended after the ordered ones
    let d = app
        .post(
            "/api/v1/devices",
            &json!({"name": "Mine", "kind": "download", "format": "epub"}),
        )
        .await
        .json();
    let last = app.get("/api/v1/devices").await.json();
    assert_eq!(last.as_array().unwrap().last().unwrap()["id"], d["id"]);
    // unknown or duplicate ids are refused
    assert_eq!(
        app.put("/api/v1/devices/order", &json!({"ids": [99999]}))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.put("/api/v1/devices/order", &json!({"ids": [ids[0], ids[0]]}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    // another user keeps the default order
    app.post(
        "/api/v1/users",
        &json!({"username": "eve", "password": "eve-pass", "role": "reader"}),
    )
    .await;
    app.login("eve", "eve-pass").await;
    let eve: Vec<i64> = app
        .get("/api/v1/devices")
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_i64().unwrap())
        .collect();
    assert_eq!(eve, ids);
}
