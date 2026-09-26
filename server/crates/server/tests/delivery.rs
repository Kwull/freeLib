//! Delivery: batched Send to Kindle, retries and their classification, per-book states,
//! persisted jobs (resume after a restart, retry), series sends, phone hand-off links,
//! device presets and Calibre metadata arguments.

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestApp, make_library};
use serde_json::{Value, json};

/// Scripted SMTP server: every mail's reply after DATA is taken from `data_replies` ("250 …"
/// when empty); `rcpt_reply` answers RCPT TO. Records the messages it accepted.
struct FakeSmtp {
    port: u16,
    msgs: Arc<Mutex<Vec<String>>>,
    data_replies: Arc<Mutex<VecDeque<String>>>,
    attempts: Arc<Mutex<usize>>,
}

async fn fake_smtp(rcpt_reply: &'static str) -> FakeSmtp {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let msgs = Arc::new(Mutex::new(Vec::new()));
    let data_replies = Arc::new(Mutex::new(VecDeque::new()));
    let attempts = Arc::new(Mutex::new(0));
    let (m2, r2, a2) = (msgs.clone(), data_replies.clone(), attempts.clone());
    tokio::spawn(async move {
        let mut queue_id = 0;
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                break;
            };
            let (msgs, replies, attempts) = (m2.clone(), r2.clone(), a2.clone());
            queue_id += 1;
            let qid = queue_id;
            tokio::spawn(async move {
                let (r, mut w) = sock.into_split();
                let mut lines = BufReader::new(r).lines();
                let _ = w.write_all(b"220 fake ESMTP\r\n").await;
                let mut data: Option<String> = None;
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(d) = data.as_mut() {
                        if line == "." {
                            *attempts.lock().unwrap() += 1;
                            let reply = replies
                                .lock()
                                .unwrap()
                                .pop_front()
                                .unwrap_or_else(|| format!("250 2.0.0 Ok: queued as Q{qid}"));
                            if reply.starts_with('2') {
                                msgs.lock().unwrap().push(data.take().unwrap());
                            }
                            data = None;
                            let _ = w.write_all(format!("{reply}\r\n").as_bytes()).await;
                        } else {
                            d.push_str(&line);
                            d.push('\n');
                        }
                        continue;
                    }
                    let cmd = line.to_ascii_uppercase();
                    let reply: String = if cmd.starts_with("EHLO") {
                        "250-fake\r\n250 8BITMIME".into()
                    } else if cmd.starts_with("RCPT") {
                        rcpt_reply.into()
                    } else if cmd.starts_with("DATA") {
                        data = Some(String::new());
                        "354 go ahead".into()
                    } else if cmd.starts_with("QUIT") {
                        let _ = w.write_all(b"221 bye\r\n").await;
                        break;
                    } else {
                        "250 OK".into()
                    };
                    let _ = w.write_all(format!("{reply}\r\n").as_bytes()).await;
                }
            });
        }
    });
    FakeSmtp {
        port,
        msgs,
        data_replies,
        attempts,
    }
}

async fn fb2_books(app: &TestApp, lib: i64) -> Vec<Value> {
    app.get(&format!(
        "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5000"
    ))
    .await
    .json()["books"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["ext"] == "fb2" && b["deleted"] == false)
        .cloned()
        .collect()
}

async fn device(app: &TestApp, preset: &str) -> Value {
    app.get("/api/v1/devices")
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["preset"] == preset)
        .unwrap_or_else(|| panic!("no {preset} device"))
        .clone()
}

async fn smtp_settings(app: &TestApp, port: u16, extra: Value) {
    let mut smtp = json!({"host": "127.0.0.1", "port": port, "security": "none", "username": "",
                          "from": "lib@example.com", "pauseSeconds": 0, "retryDelaySeconds": 0});
    for (k, v) in extra.as_object().unwrap() {
        smtp[k] = v.clone();
    }
    let r = app.put("/api/v1/settings", &json!({ "smtp": smtp })).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
}

async fn app_with_library() -> (TestApp, i64) {
    let app = TestApp::new(|_, root| {
        make_library(root, 60);
    })
    .await;
    let lib = app.import_library().await["id"].as_i64().unwrap();
    (app, lib)
}

#[tokio::test]
async fn batched_send_with_per_book_states_and_kindle_hint() {
    let smtp = fake_smtp("250 OK").await;
    let (app, lib) = app_with_library().await;
    smtp_settings(&app, smtp.port, json!({"maxAttachments": 2})).await;
    let kindle = device(&app, "kindle-email").await;
    let ids: Vec<i64> = fb2_books(&app, lib).await[..3]
        .iter()
        .map(|b| b["id"].as_i64().unwrap())
        .collect();
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": ids, "device": kindle["id"], "target": "me@kindle.com"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let job = r.json();
    assert_eq!(job["items"].as_array().unwrap().len(), 3);
    assert!(
        job["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["state"] == "queued")
    );
    let done = app.wait_job(job["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["message"], "3 books sent in 2 e-mails");
    let msgs = smtp.msgs.lock().unwrap().clone();
    assert_eq!(msgs.len(), 2, "25-per-mail limit lowered to 2: 2 + 1");
    assert_eq!(msgs[0].matches("application/epub+zip").count(), 2);
    let items = done["items"].as_array().unwrap();
    let mails: Vec<i64> = items.iter().map(|i| i["mail"].as_i64().unwrap()).collect();
    assert_eq!(mails, [1, 1, 2]);
    for i in items {
        assert_eq!(i["state"], "accepted", "{i}");
        assert!(i["detail"].as_str().unwrap().contains("queued as Q"), "{i}");
        assert_eq!(i["attempts"], 1);
        assert!(i["size"].as_u64().unwrap() > 1000);
    }
    assert_eq!(done["hint"]["code"], "kindle_approved_sender");
    assert_eq!(done["hint"]["from"], "lib@example.com");
    assert!(
        done["hint"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://www.amazon.")
    );
    // a non-Kindle recipient gets no hint
    smtp_settings(&app, smtp.port, json!({"allowedRecipients": ["*"]})).await;
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": [ids[0]], "device": kindle["id"], "target": "me@example.com"}),
        )
        .await;
    let done = app.wait_job(r.json()["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done");
    assert!(done["hint"].is_null());
}

#[tokio::test]
async fn retries_temporary_failures_only() {
    let smtp = fake_smtp("250 OK").await;
    let (app, lib) = app_with_library().await;
    smtp_settings(&app, smtp.port, json!({"retries": 2})).await;
    let kindle = device(&app, "kindle-email").await;
    let id = fb2_books(&app, lib).await[0]["id"].as_i64().unwrap();
    let body =
        json!({"library": lib, "books": [id], "device": kindle["id"], "target": "me@kindle.com"});

    // 4xx: retried, then accepted
    smtp.data_replies
        .lock()
        .unwrap()
        .push_back("451 4.3.0 Temporary local problem".into());
    let r = app.post("/api/v1/send", &body).await;
    let done = app.wait_job(r.json()["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    let it = &done["items"][0];
    assert_eq!(it["state"], "accepted");
    assert_eq!(it["attempts"], 2, "{it}");
    assert!(
        done["log"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().contains("retry 1 of 2"))
    );
    assert_eq!(*smtp.attempts.lock().unwrap(), 2);

    // 5xx: permanent, no retry
    *smtp.attempts.lock().unwrap() = 0;
    smtp.data_replies
        .lock()
        .unwrap()
        .push_back("554 5.7.1 Message rejected".into());
    let r = app.post("/api/v1/send", &body).await;
    let done = app.wait_job(r.json()["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(done["items"][0]["state"], "failed");
    assert_eq!(done["items"][0]["attempts"], 1);
    assert!(done["items"][0]["detail"].as_str().unwrap().contains("554"));
    assert!(done["message"].as_str().unwrap().contains("554"), "{done}");
    assert_eq!(
        done["retryable"], true,
        "a failed job can be retried by hand"
    );
    assert_eq!(*smtp.attempts.lock().unwrap(), 1);

    // retries exhausted: 4xx every time → failed after 1 + 2 attempts
    *smtp.attempts.lock().unwrap() = 0;
    for _ in 0..3 {
        smtp.data_replies
            .lock()
            .unwrap()
            .push_back("421 4.7.0 Try again later".into());
    }
    let r = app.post("/api/v1/send", &body).await;
    let done = app.wait_job(r.json()["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "failed");
    assert_eq!(done["items"][0]["attempts"], 3);
    assert_eq!(*smtp.attempts.lock().unwrap(), 3);

    // a manual retry of that job now goes through, and the book is not sent twice later
    let jid = done["id"].as_str().unwrap().to_string();
    let r = app.post_empty(&format!("/api/v1/jobs/{jid}/retry")).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let done = app.wait_job(&jid).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["items"][0]["state"], "accepted");
    let r = app.post_empty(&format!("/api/v1/jobs/{jid}/retry")).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "nothing left to retry");

    // a rejected recipient is permanent too
    let smtp2 = fake_smtp("550 5.1.1 No such user").await;
    smtp_settings(&app, smtp2.port, json!({})).await;
    let r = app.post("/api/v1/send", &body).await;
    let done = app.wait_job(r.json()["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "failed");
    assert_eq!(done["items"][0]["attempts"], 1);
    assert!(done["items"][0]["detail"].as_str().unwrap().contains("550"));

    // a server that is not there: a network error, retried
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    smtp_settings(&app, port, json!({"retries": 1})).await;
    let r = app.post("/api/v1/send", &body).await;
    let done = app.wait_job(r.json()["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "failed");
    assert_eq!(done["items"][0]["attempts"], 2, "{done}");
}

#[tokio::test]
async fn whole_series_in_few_mails() {
    let smtp = fake_smtp("250 OK").await;
    let (app, lib) = app_with_library().await;
    smtp_settings(&app, smtp.port, json!({})).await;
    let books = fb2_books(&app, lib).await;
    // the series with the most live FB2 books
    let mut counts: std::collections::HashMap<i64, Vec<&Value>> = Default::default();
    for b in &books {
        if let Some(s) = b["series"]["id"].as_i64() {
            counts.entry(s).or_default().push(b);
        }
    }
    let (sid, members) = counts.into_iter().max_by_key(|(_, v)| v.len()).unwrap();
    assert!(members.len() >= 2, "synthetic library has a series");
    let kindle = device(&app, "kindle-email").await;
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "series": [sid], "device": kindle["id"], "target": "me@kindle.com"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let job = r.json();
    let n = job["items"].as_array().unwrap().len();
    assert!(n >= members.len().min(2), "{job}");
    assert!(job["title"].as_str().unwrap().contains('«'), "{job}");
    let done = app.wait_job(job["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(
        smtp.msgs.lock().unwrap().len(),
        1,
        "one mail for the series"
    );
    // unknown series → 404
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "series": [999_999], "device": kindle["id"], "target": "me@kindle.com"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn jobs_survive_a_restart() {
    let (app, lib) = app_with_library().await;
    let kobo = device(&app, "kobo").await;
    let ids: Vec<i64> = fb2_books(&app, lib).await[..2]
        .iter()
        .map(|b| b["id"].as_i64().unwrap())
        .collect();
    // a finished download job
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": [ids[0]], "device": kobo["id"]}),
        )
        .await;
    let finished = app.wait_job(r.json()["id"].as_str().unwrap()).await;
    assert_eq!(finished["state"], "done");
    app.state.jobs.flush().await;
    // a queued and a running job, as a crash would leave them
    let dev = kobo.clone();
    let request =
        json!({"library": lib, "books": ids, "device": dev, "target": null, "file_name": null})
            .to_string();
    {
        let c = app.state.db.lock();
        for (id, state) in [
            ("aaaa000000000001", "queued"),
            ("aaaa000000000002", "running"),
        ] {
            c.execute(
                "INSERT INTO job(id, owner, kind, title, state, request, created_at) VALUES (?1, 0, 'download', 'Download for Kobo · 2 books', ?2, ?3, '2026-01-01T00:00:00Z')",
                rusqlite::params![id, state, request],
            )
            .unwrap();
            for (pos, b) in ids.iter().enumerate() {
                c.execute(
                    "INSERT INTO job_item(job_id, pos, book_id, title, state, updated_at) VALUES (?1, ?2, ?3, 'x', ?4, 't')",
                    rusqlite::params![id, pos as i64, b, if pos == 0 { "ready" } else { "converting" }],
                )
                .unwrap();
            }
        }
    }
    let app = app.restart().await;
    // the queued job was resumed
    let resumed = app.wait_job("aaaa000000000001").await;
    assert_eq!(resumed["state"], "done", "{resumed}");
    assert!(resumed["downloadUrl"].is_string());
    let f = app.get(resumed["downloadUrl"].as_str().unwrap()).await;
    assert_eq!(f.status, StatusCode::OK);
    assert_eq!(f.header("content-type"), "application/zip");
    // the running one is failed and retryable
    let jobs = app.get("/api/v1/jobs").await.json();
    let interrupted = jobs
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == "aaaa000000000002")
        .unwrap()
        .clone();
    assert_eq!(interrupted["state"], "failed");
    assert_eq!(interrupted["retryable"], true);
    assert!(interrupted["message"].as_str().unwrap().contains("restart"));
    assert_eq!(interrupted["items"][1]["state"], "failed");
    // the finished job and its file are still there
    let old = jobs
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == finished["id"])
        .expect("finished job kept");
    assert_eq!(old["state"], "done");
    let f = app.get(old["downloadUrl"].as_str().unwrap()).await;
    assert_eq!(f.status, StatusCode::OK);
    // retry the interrupted one
    let r = app.post_empty("/api/v1/jobs/aaaa000000000002/retry").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let done = app.wait_job("aaaa000000000002").await;
    assert_eq!(done["state"], "done", "{done}");
    assert!(
        done["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["state"] == "ready")
    );
    // clearing finished jobs removes them for good
    assert_eq!(
        app.delete("/api/v1/jobs?finished=1").await.status,
        StatusCode::NO_CONTENT
    );
    app.state.jobs.flush().await;
    let n: i64 = app
        .state
        .db
        .lock()
        .query_row("SELECT count(*) FROM job", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 0);
}

fn get_with(path: &str, method: Method, ua: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::USER_AGENT, ua)
        .header(header::ACCEPT_LANGUAGE, "ru-RU,ru;q=0.9")
        .body(Body::empty())
        .unwrap()
}

const IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";

#[tokio::test]
async fn phone_handoff_links() {
    let (mut app, lib) = app_with_library().await;
    let book = fb2_books(&app, lib).await[0].clone();
    let id = book["id"].as_i64().unwrap();
    let r = app
        .post("/api/v1/handoff", &json!({"library": lib, "book": id}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let link = r.json();
    assert_eq!(link["format"], "epub", "the Apple Books device by default");
    assert_eq!(link["maxUses"], 3);
    let url = link["url"].as_str().unwrap().to_string();
    let token = url.trim_start_matches("/h/").to_string();
    assert_eq!(token.len(), 22);
    assert!(link["absoluteUrl"].as_str().unwrap().ends_with(&url));
    // only the hash is stored
    let stored: i64 = app
        .state
        .db
        .lock()
        .query_row(
            "SELECT count(*) FROM handoff WHERE token_hash LIKE ?1",
            [format!("%{token}%")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, 0);

    // the page works without a session (log out first: open mode has no session anyway)
    app.cookie = None;
    let page = app.send(get_with(&url, Method::GET, IPHONE)).await;
    assert_eq!(page.status, StatusCode::OK);
    let html = page.text();
    assert!(html.contains("Открыть в Книгах"), "{html}");
    assert!(html.contains(&format!("/h/{token}/file")));
    assert!(
        page.header("content-security-policy")
            .contains("default-src 'none'")
    );
    assert!(!html.contains("<script"));
    assert_eq!(page.header("cache-control"), "no-store");
    assert_eq!(page.header("referrer-policy"), "no-referrer");
    let desktop = app
        .send(get_with(
            &url,
            Method::GET,
            "Mozilla/5.0 (X11; Linux x86_64)",
        ))
        .await;
    assert!(desktop.text().contains("Скачать"));
    let cover = app.get(&format!("{url}/cover")).await;
    assert_eq!(cover.status, StatusCode::OK);

    // HEAD does not use up the link; three downloads do
    let head = app
        .send(get_with(&format!("{url}/file"), Method::HEAD, IPHONE))
        .await;
    assert_eq!(head.status, StatusCode::OK);
    for _ in 0..3 {
        let f = app
            .send(get_with(&format!("{url}/file"), Method::GET, IPHONE))
            .await;
        assert_eq!(f.status, StatusCode::OK);
        assert_eq!(f.header("content-type"), "application/epub+zip");
        let cd = f.header("content-disposition");
        assert!(
            cd.starts_with("attachment;") && cd.contains(".epub"),
            "{cd}"
        );
        assert!(f.body.starts_with(b"PK"));
    }
    let f = app
        .send(get_with(&format!("{url}/file"), Method::GET, IPHONE))
        .await;
    assert_eq!(f.status, StatusCode::GONE, "reuse limit");
    assert!(f.text().contains("использована"));

    // tampered and unknown tokens
    let mut bad = token.clone();
    bad.replace_range(0..1, if token.starts_with('A') { "B" } else { "A" });
    for p in [
        format!("/h/{bad}"),
        format!("/h/{bad}/file"),
        "/h/short".into(),
    ] {
        let r = app.send(get_with(&p, Method::GET, IPHONE)).await;
        assert!(
            r.status == StatusCode::GONE || r.status == StatusCode::NOT_FOUND,
            "{p}: {}",
            r.status
        );
        assert!(!r.body.starts_with(b"PK"));
    }

    // expiry
    let r = app
        .post(
            "/api/v1/handoff",
            &json!({"library": lib, "book": id, "device": device(&app, "kobo").await["id"]}),
        )
        .await
        .json();
    assert_eq!(r["format"], "kepub");
    let url2 = r["url"].as_str().unwrap().to_string();
    app.state
        .db
        .lock()
        .execute("UPDATE handoff SET expires_at = expires_at - 3600", [])
        .unwrap();
    let r = app.send(get_with(&url2, Method::GET, IPHONE)).await;
    assert_eq!(r.status, StatusCode::GONE);
    assert!(r.text().contains("устарела"));
    let r = app
        .send(get_with(&format!("{url2}/file"), Method::GET, IPHONE))
        .await;
    assert_eq!(r.status, StatusCode::GONE);

    // creation is rate limited
    let mut limited = false;
    for _ in 0..25 {
        let r = app
            .post("/api/v1/handoff", &json!({"library": lib, "book": id}))
            .await;
        if r.status == StatusCode::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
        assert_eq!(r.status, StatusCode::OK);
    }
    assert!(limited);
    // unknown book / device
    let r = app
        .post(
            "/api/v1/handoff",
            &json!({"library": lib, "book": 99_999_999}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn presets_and_customization() {
    let (app, _lib) = app_with_library().await;
    let devs = app.get("/api/v1/devices").await.json();
    let presets: Vec<&str> = devs
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["preset"].as_str().unwrap())
        .collect();
    assert_eq!(
        presets,
        [
            "kindle-email",
            "kindle-usb",
            "apple-books",
            "kobo",
            "server-folder",
            "original"
        ]
    );
    let kindle = device(&app, "kindle-email").await;
    assert_eq!(kindle["options"]["footnotes"], "popup");
    assert_eq!(kindle["options"]["hyphenate"], "soft");
    assert!(kindle["options"]["fontFamily"].is_null());
    // the preset is read-only and survives edits
    let mut d = kindle.clone();
    d["preset"] = json!("kobo");
    d["target"] = json!("me@kindle.com");
    let r = app
        .put(&format!("/api/v1/devices/{}", kindle["id"]), &d)
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["preset"], "kindle-email");
    let customized: i64 = app
        .state
        .db
        .lock()
        .query_row(
            "SELECT customized FROM device WHERE id=?1",
            [kindle["id"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(customized, 0, "a new target is not a customization");
    d["options"]["dropCaps"] = json!(true);
    app.put(&format!("/api/v1/devices/{}", kindle["id"]), &d)
        .await;
    let customized: i64 = app
        .state
        .db
        .lock()
        .query_row(
            "SELECT customized FROM device WHERE id=?1",
            [kindle["id"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(customized, 1);
    // devices users create have no preset
    let r = app
        .post(
            "/api/v1/devices",
            &json!({"name": "Mine", "kind": "download", "format": "epub", "preset": "kobo"}),
        )
        .await
        .json();
    assert!(r["preset"].is_null());
}

/// A fake `ebook-convert` that records its arguments next to the output.
fn recording_calibre(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("ebook-convert");
    let log = dir.join("calibre-args.txt");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'ebook-convert (calibre 7.4.0)'; exit 0; fi\n\
             for a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{}'\ncp \"$1\" \"$2\"\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

#[tokio::test]
async fn calibre_gets_the_book_metadata() {
    let app = TestApp::new(|cfg, root| {
        make_library(root, 60);
        cfg.calibre = Some(recording_calibre(root));
    })
    .await;
    let lib = app.import_library().await["id"].as_i64().unwrap();
    let books = fb2_books(&app, lib).await;
    let b = books
        .iter()
        .find(|b| b["series"].is_object() && b["serno"].as_i64().is_some_and(|n| n > 0))
        .expect("a book in a series");
    let f = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{}/file?format=azw3",
            b["id"]
        ))
        .await;
    assert_eq!(f.status, StatusCode::OK, "{}", f.text());
    let args = std::fs::read_to_string(app.root().join("calibre-args.txt")).unwrap();
    let lines: Vec<&str> = args.lines().collect();
    assert!(lines[0].ends_with("in.epub") && lines[1].ends_with("out.azw3"));
    let has = |p: &str| lines.iter().any(|l| l.starts_with(p));
    for p in [
        "--title=",
        "--title-sort=",
        "--authors=",
        "--author-sort=",
        "--language=",
        "--cover=",
    ] {
        assert!(has(p), "{p} missing: {lines:?}");
    }
    assert!(
        lines.contains(&format!("--series={}", b["series"]["name"].as_str().unwrap()).as_str()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&format!("--series-index={}", b["serno"]).as_str()),
        "{lines:?}"
    );
}
