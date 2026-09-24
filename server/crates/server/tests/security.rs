//! Regression tests of the security review fixes (docs/web/API.md, DOCKER.md).

mod common;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestApp, fake_calibre, make_library};
use serde_json::{Value, json};

async fn first_fb2(app: &TestApp, lib: i64) -> i64 {
    let r = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5000"
        ))
        .await
        .json();
    r["books"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["ext"] == "fb2")
        .unwrap()["id"]
        .as_i64()
        .unwrap()
}

async fn fb2_ids(app: &TestApp, lib: i64, n: usize) -> Vec<i64> {
    let r = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5000"
        ))
        .await
        .json();
    r["books"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["ext"] == "fb2")
        .take(n)
        .map(|b| b["id"].as_i64().unwrap())
        .collect()
}

fn with_host(path: &str, host: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header(header::HOST, host)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn csp_inline_and_file_headers() {
    let app = TestApp::new(|_, root| {
        make_library(root, 50);
    })
    .await;
    let lib = app.import_library().await["id"].as_i64().unwrap();
    let id = first_fb2(&app, lib).await;

    // app / API responses carry the strict policy
    let s = app.get("/api/v1/session").await;
    let csp = s.header("content-security-policy");
    assert!(csp.contains("script-src 'self'"), "{csp}");
    assert!(csp.contains("object-src 'none'"), "{csp}");
    assert!(csp.contains("frame-ancestors 'self'"), "{csp}");

    // inline=1 is honoured for EPUB only
    let orig = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=original&inline=1"
        ))
        .await;
    assert_eq!(orig.status, StatusCode::OK);
    assert!(
        orig.header("content-disposition")
            .starts_with("attachment;")
    );
    // FB2 is XML: never served with a type a browser renders
    assert_eq!(orig.header("content-type"), "application/octet-stream");
    assert_eq!(orig.header("content-security-policy"), "sandbox");
    assert_eq!(orig.header("x-content-type-options"), "nosniff");
    let epub = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=epub&inline=1"
        ))
        .await;
    assert!(epub.header("content-disposition").starts_with("inline;"));
    assert_eq!(epub.header("content-type"), "application/epub+zip");
    assert_eq!(epub.header("content-security-policy"), "sandbox");
    let kepub = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=kepub&inline=1"
        ))
        .await;
    assert_eq!(kepub.status, StatusCode::OK);
    // kepub ends in .epub too, but only the reader's format=epub is meant inline; both are EPUB
    assert_eq!(kepub.header("content-type"), "application/epub+zip");

    // covers (real and placeholder)
    let cover = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/cover?size=thumb"
        ))
        .await;
    assert_eq!(cover.status, StatusCode::OK);
    assert_eq!(cover.header("content-security-policy"), "sandbox");
    assert_eq!(cover.header("x-content-type-options"), "nosniff");
}

#[tokio::test]
async fn dns_rebinding_guard() {
    // open mode: only localhost, IP literals and FREELIB_ALLOWED_HOSTS
    let app = TestApp::new(|cfg, _| {
        cfg.allowed_hosts = vec!["books.example.org".into()];
    })
    .await;
    for h in [
        "localhost:8080",
        "127.0.0.1:8080",
        "[::1]:8080",
        "books.example.org",
    ] {
        let r = app.send(with_host("/api/v1/session", h)).await;
        assert_eq!(r.status, StatusCode::OK, "{h}");
    }
    for h in ["attacker.example", "books.example.org.attacker.example"] {
        for p in ["/api/v1/session", "/", "/opds"] {
            let r = app.send(with_host(p, h)).await;
            assert_eq!(r.status, StatusCode::MISDIRECTED_REQUEST, "{h}{p}");
        }
    }
    // open mode without a list: still only localhost / IPs
    let app = TestApp::new(|_, _| {}).await;
    assert_eq!(
        app.send(with_host("/api/v1/libraries", "rebind.attacker.example"))
            .await
            .status,
        StatusCode::MISDIRECTED_REQUEST
    );
    // login mode without a list: any host (the reverse proxy decides)
    let app = TestApp::new(|cfg, _| {
        cfg.admin_password = Some("pw-admin".into());
    })
    .await;
    assert_eq!(
        app.send(with_host("/api/v1/session", "books.example.net"))
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn login_limits_per_user_and_proxy_ip() {
    let mut app = TestApp::new(|cfg, _| {
        cfg.admin_password = Some("pw-admin".into());
        cfg.trust_proxy = true;
    })
    .await;
    let attempt = |ip: &str, user: &str, pw: &str| {
        Request::builder()
            .method(Method::POST)
            .uri("/api/v1/login")
            .header(header::CONTENT_TYPE, "application/json")
            // a client-supplied first entry must not be used as the address
            .header("x-forwarded-for", format!("1.1.1.1, {ip}"))
            .body(Body::from(
                json!({"username": user, "password": pw}).to_string(),
            ))
            .unwrap()
    };
    // 5 failures for "admin" from rotating addresses → the user name is throttled
    for i in 0..5 {
        let r = app
            .send(attempt(&format!("10.0.0.{i}"), "admin", "wrong"))
            .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    let r = app.send(attempt("10.0.0.99", "ADMIN", "pw-admin")).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "{}", r.text());
    // other users from a fresh address are not affected by that
    let r = app.send(attempt("10.0.1.1", "nobody", "x")).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // IP throttling uses the rightmost X-Forwarded-For entry: 1.1.1.1 was never blocked
    for _ in 0..5 {
        app.send(attempt("10.9.9.9", "u1", "x")).await;
    }
    let r = app.send(attempt("10.9.9.9", "u2", "x")).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    let r = app.send(attempt("10.9.9.8", "u3", "x")).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    let _ = app.login("x", "y").await;
}

#[tokio::test]
async fn mail_recipients_and_daily_limit() {
    let (port, msgs) = common_smtp().await;
    let app = TestApp::new(|_, root| {
        make_library(root, 50);
    })
    .await;
    let lib = app.import_library().await["id"].as_i64().unwrap();
    let s = app.get("/api/v1/settings").await.json();
    assert_eq!(
        s["smtp"]["allowedRecipients"],
        json!(["*@kindle.com", "*@free.kindle.com"])
    );
    assert_eq!(s["smtp"]["dailyLimitPerUser"], 100);
    let r = app
        .put(
            "/api/v1/settings",
            &json!({"smtp": {"host": "127.0.0.1", "port": port, "security": "none", "from": "lib@example.com",
                             "pauseSeconds": 0, "dailyLimitPerUser": 2}}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(
        app.put(
            "/api/v1/settings",
            &json!({"smtp": {"allowedRecipients": ["no-at-sign"]}})
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    let kindle = app.get("/api/v1/devices").await.json()[0]["id"].clone();
    let ids = fb2_ids(&app, lib, 3).await;
    // not an allowed recipient
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": [ids[0]], "device": kindle, "target": "victim@example.com"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    assert!(r.json()["message"].as_str().unwrap().contains("allowed"));
    // devices cannot store one either
    let r = app
        .post(
            "/api/v1/devices",
            &json!({"name": "x", "kind": "email", "format": "epub", "target": "victim@example.com"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // allowed recipient; the daily limit (2) counts every mail
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": [ids[0], ids[1]], "device": kindle, "target": "me@Kindle.com"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let done = app.wait_job(r.json()["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(msgs.lock().unwrap().len(), 2);
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": [ids[2]], "device": kindle, "target": "me@kindle.com"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "{}", r.text());
    // an admin can widen the list
    let r = app
        .put(
            "/api/v1/settings",
            &json!({"smtp": {"allowedRecipients": ["*@example.com"], "dailyLimitPerUser": 10}}),
        )
        .await
        .json();
    assert_eq!(r["smtp"]["allowedRecipients"], json!(["*@example.com"]));
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": [ids[2]], "device": kindle, "target": "me@example.com"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
}

/// Minimal SMTP sink (see api.rs for the full one).
async fn common_smtp() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let msgs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let m2 = msgs.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let msgs = m2.clone();
            tokio::spawn(async move {
                let (r, mut w) = sock.into_split();
                let mut lines = BufReader::new(r).lines();
                let _ = w.write_all(b"220 fake ESMTP\r\n").await;
                let mut data: Option<String> = None;
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(d) = data.as_mut() {
                        if line == "." {
                            msgs.lock().unwrap().push(data.take().unwrap());
                            let _ = w.write_all(b"250 OK\r\n").await;
                        } else {
                            d.push_str(&line);
                            d.push('\n');
                        }
                        continue;
                    }
                    let cmd = line.to_ascii_uppercase();
                    let reply: &[u8] = if cmd.starts_with("EHLO") {
                        b"250-fake\r\n250 8BITMIME\r\n"
                    } else if cmd.starts_with("DATA") {
                        data = Some(String::new());
                        b"354 go ahead\r\n"
                    } else if cmd.starts_with("QUIT") {
                        let _ = w.write_all(b"221 bye\r\n").await;
                        break;
                    } else {
                        b"250 OK\r\n"
                    };
                    let _ = w.write_all(reply).await;
                }
            });
        }
    });
    (port, msgs)
}

#[tokio::test]
async fn users_jobs_and_folders() {
    let mut app = TestApp::new(|cfg, root| {
        make_library(root, 300);
        cfg.admin_password = Some("pw-admin".into());
        cfg.max_jobs_per_user = 1;
    })
    .await;
    app.login("admin", "pw-admin").await;
    let lib = app.import_library().await["id"].as_i64().unwrap();
    let admin_cookie = app.cookie.clone();

    // user names are unique case-insensitively
    let r = app
        .post(
            "/api/v1/users",
            &json!({"username": "Reader", "password": "pw-reader", "role": "reader"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let reader_id = r.json()["id"].as_i64().unwrap();
    assert_eq!(
        app.post(
            "/api/v1/users",
            &json!({"username": "READER", "password": "pw-reader"})
        )
        .await
        .status,
        StatusCode::CONFLICT
    );

    app.cookie = None;
    assert_eq!(
        app.login("reader", "pw-reader").await.status,
        StatusCode::OK
    );
    // readers cannot create server folder devices …
    let r = app
        .post(
            "/api/v1/devices",
            &json!({"name": "mine", "kind": "folder", "format": "epub", "target": "x"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let devices = app.get("/api/v1/devices").await.json();
    let find = |name: &str| {
        devices
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == name)
            .unwrap()["id"]
            .clone()
    };
    let folder = find("Server folder");
    let original = find("Original");
    let ids = fb2_ids(&app, lib, 200).await;
    // … nor pick another folder of a shared one
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": [ids[0]], "device": folder, "target": "elsewhere"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // but may use it as configured; exports never overwrite
    for _ in 0..2 {
        let r = app
            .post(
                "/api/v1/send",
                &json!({"library": lib, "books": [ids[0]], "device": folder}),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        let done = app.wait_job(r.json()["id"].as_str().unwrap()).await;
        assert_eq!(done["state"], "done", "{done}");
    }
    let mut files: Vec<String> = walk(&app.state.cfg.export_dir);
    files.sort();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(
        files[1].ends_with(" (2).epub") || files[0].ends_with(" (2).epub"),
        "{files:?}"
    );

    // per-user cap on queued + running jobs (1 in this test)
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": ids, "device": original}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let big = r.json()["id"].as_str().unwrap().to_string();
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": [ids[0]], "device": original}),
        )
        .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS, "{}", r.text());
    assert_eq!(r.json()["error"], "rate_limited");
    app.post_empty(&format!("/api/v1/jobs/{big}/cancel")).await;
    let j = app.wait_job(&big).await;
    assert_ne!(j["state"], "running");

    // deleting the reader purges its jobs; the next user never gets its id
    app.cookie = admin_cookie;
    assert_eq!(
        app.delete(&format!("/api/v1/users/{reader_id}"))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    let r = app
        .post(
            "/api/v1/users",
            &json!({"username": "next", "password": "pw-next", "role": "reader"}),
        )
        .await
        .json();
    assert!(r["id"].as_i64().unwrap() > reader_id);
    app.cookie = None;
    app.login("next", "pw-next").await;
    assert_eq!(app.get("/api/v1/jobs").await.json(), json!([]));
    let admin = app.state.jobs.list(&freelib_server::db::User {
        id: reader_id,
        username: "reader".into(),
        role: "reader".into(),
    });
    assert!(admin.is_empty(), "jobs of the deleted user are gone");
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p.to_string_lossy().into_owned());
            }
        }
    }
    out
}

#[tokio::test]
async fn calibre_gets_epub_only() {
    let app = TestApp::new(|cfg, root| {
        make_library(root, 100);
        cfg.calibre = Some(fake_calibre(root));
    })
    .await;
    let lib = app.import_library().await["id"].as_i64().unwrap();
    let r = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5000"
        ))
        .await
        .json();
    let other: Vec<Value> = r["books"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["ext"] != "fb2" && b["ext"] != "epub")
        .cloned()
        .collect();
    assert!(
        !other.is_empty(),
        "the synthetic library has some PDF/DjVu books"
    );
    for b in other.iter().take(3) {
        let d = app
            .get(&format!("/api/v1/libraries/{lib}/books/{}", b["id"]))
            .await
            .json();
        assert_eq!(d["formats"], json!(["original"]), "{}", b["ext"]);
        for f in ["epub", "azw3", "pdf"] {
            let r = app
                .get(&format!(
                    "/api/v1/libraries/{lib}/books/{}/file?format={f}",
                    b["id"]
                ))
                .await;
            assert_eq!(r.status, StatusCode::NOT_IMPLEMENTED);
        }
    }
}

#[tokio::test]
async fn sse_closes_for_deleted_user() {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let mut app = TestApp::new(|cfg, _| {
        cfg.admin_password = Some("pw-admin".into());
    })
    .await;
    app.login("admin", "pw-admin").await;
    let admin_cookie = app.cookie.clone();
    let r = app
        .post(
            "/api/v1/users",
            &json!({"username": "sse", "password": "pw-sse", "role": "reader"}),
        )
        .await
        .json();
    let id = r["id"].as_i64().unwrap();
    app.cookie = None;
    app.login("sse", "pw-sse").await;
    let req = app.request(Method::GET, "/api/v1/events", None);
    let resp = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();
    let ended = tokio::spawn(async move {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(10), body.frame()).await {
                Ok(None) | Ok(Some(Err(_))) => return true,
                Ok(Some(Ok(_))) => continue,
                Err(_) => return false,
            }
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    app.cookie = admin_cookie;
    assert_eq!(
        app.delete(&format!("/api/v1/users/{id}")).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(
        ended.await.unwrap(),
        "the event stream of a deleted user ends"
    );
}
