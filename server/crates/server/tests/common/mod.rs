//! Test harness: a router over a temp dir with a small generated library.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use freelib_server::{AppState, Config};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

pub struct TestApp {
    pub dir: tempfile::TempDir,
    pub state: AppState,
    pub router: Router,
    pub cookie: Option<String>,
}

pub struct Resp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }
    pub fn header(&self, name: &str) -> String {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Generates `books` synthetic books with FB2 archives under `<root>/books/lib`.
pub fn make_library(root: &Path, books: usize) -> PathBuf {
    let lib = root.join("books/lib");
    std::fs::create_dir_all(&lib).unwrap();
    let opts = freelib_import::synth::GenOptions {
        books,
        per_archive: 100,
        seed: 7,
        files_dir: Some(lib.clone()),
        structure_info: true,
    };
    freelib_import::synth::generate(&lib.join("test.inpx"), &opts).unwrap();
    lib
}

impl TestApp {
    pub async fn new(customize: impl FnOnce(&mut Config, &Path)) -> TestApp {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::for_dir(dir.path());
        std::fs::create_dir_all(&cfg.books_dir).unwrap();
        customize(&mut cfg, dir.path());
        let state = freelib_server::init(cfg).await.unwrap();
        let router = freelib_server::router(state.clone());
        TestApp {
            dir,
            state,
            router,
            cookie: None,
        }
    }

    /// Stops this app and starts a fresh one over the same directories (default config),
    /// like a server restart.
    pub async fn restart(self) -> TestApp {
        let TestApp {
            dir, state, router, ..
        } = self;
        drop(router);
        drop(state);
        let cfg = Config::for_dir(dir.path());
        let state = freelib_server::init(cfg).await.unwrap();
        let router = freelib_server::router(state.clone());
        TestApp {
            dir,
            state,
            router,
            cookie: None,
        }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub async fn send(&self, req: Request<Body>) -> Resp {
        let r = self.router.clone().oneshot(req).await.unwrap();
        let status = r.status();
        let headers = r.headers().clone();
        let body = r.into_body().collect().await.unwrap().to_bytes().to_vec();
        Resp {
            status,
            headers,
            body,
        }
    }

    pub fn request(&self, method: Method, path: &str, body: Option<&Value>) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(path);
        if let Some(c) = &self.cookie {
            b = b.header(header::COOKIE, c);
        }
        match body {
            Some(v) => b
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(v).unwrap()))
                .unwrap(),
            None => b.body(Body::empty()).unwrap(),
        }
    }

    pub async fn get(&self, path: &str) -> Resp {
        self.send(self.request(Method::GET, path, None)).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> Resp {
        self.send(self.request(Method::POST, path, Some(body)))
            .await
    }

    pub async fn post_empty(&self, path: &str) -> Resp {
        self.send(self.request(Method::POST, path, None)).await
    }

    pub async fn put(&self, path: &str, body: &Value) -> Resp {
        self.send(self.request(Method::PUT, path, Some(body))).await
    }

    pub async fn patch(&self, path: &str, body: &Value) -> Resp {
        self.send(self.request(Method::PATCH, path, Some(body)))
            .await
    }

    pub async fn delete(&self, path: &str) -> Resp {
        self.send(self.request(Method::DELETE, path, None)).await
    }

    pub async fn login(&mut self, user: &str, pw: &str) -> Resp {
        let r = self
            .post(
                "/api/v1/login",
                &serde_json::json!({"username": user, "password": pw}),
            )
            .await;
        if r.status == StatusCode::OK {
            let c = r.header("set-cookie");
            self.cookie = Some(c.split(';').next().unwrap().to_string());
        }
        r
    }

    /// Polls `GET /jobs` until job `id` is finished; returns it.
    pub async fn wait_job(&self, id: &str) -> Value {
        for _ in 0..1200 {
            let jobs = self.get("/api/v1/jobs").await.json();
            if let Some(j) = jobs.as_array().unwrap().iter().find(|j| j["id"] == id)
                && ["done", "failed", "cancelled"].contains(&j["state"].as_str().unwrap())
            {
                return j.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("job {id} did not finish");
    }

    /// Creates library "Test" from `books/lib/test.inpx` and waits for the import.
    pub async fn import_library(&self) -> Value {
        let r = self
            .post(
                "/api/v1/libraries",
                &serde_json::json!({"name": "Test", "path": "lib", "inpx": "lib/test.inpx", "isDefault": true}),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        let lib = r.json();
        let jobs = self.get("/api/v1/jobs").await.json();
        let job = jobs
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["kind"] == "import")
            .expect("import job")
            .clone();
        let done = self.wait_job(job["id"].as_str().unwrap()).await;
        assert_eq!(done["state"], "done", "{done}");
        let libs = self.get("/api/v1/libraries").await.json();
        libs.as_array()
            .unwrap()
            .iter()
            .find(|l| l["id"] == lib["id"])
            .unwrap()
            .clone()
    }
}

/// A fake `ebook-convert`: `--version` prints a Calibre banner, otherwise copies input to output.
pub fn fake_calibre(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join("ebook-convert");
    std::fs::write(
        &p,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'ebook-convert (calibre 7.4.0)'; exit 0; fi\ncp \"$1\" \"$2\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// Checks that `xml` is well-formed (matching end tags); returns all elements with attributes.
pub fn parse_xml(xml: &[u8]) -> Vec<(String, Vec<(String, String)>)> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_reader(xml);
    let mut out = Vec::new();
    let mut buf = Vec::new();
    let mut depth = 0i32;
    loop {
        let ev = r.read_event_into(&mut buf);
        match ev {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if matches!(ev, Ok(Event::Start(_))) {
                    depth += 1;
                }
                let name = e.name().as_ref().to_string();
                let attrs = e
                    .attributes()
                    .map(|a| {
                        let a = a.expect("valid attribute");
                        (
                            a.key.as_ref().to_string(),
                            a.normalized_value(quick_xml::XmlVersion::default())
                                .expect("escaped")
                                .into_owned(),
                        )
                    })
                    .collect();
                out.push((name, attrs));
            }
            Ok(Event::End(_)) => depth -= 1,
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => panic!("XML error: {e}\n{}", String::from_utf8_lossy(xml)),
        }
        buf.clear();
    }
    assert_eq!(depth, 0, "unbalanced XML");
    out
}

/// `(rel, href, type)` of all `<link>` elements.
pub fn links(xml: &[u8]) -> Vec<(String, String, String)> {
    parse_xml(xml)
        .into_iter()
        .filter(|(n, _)| n == "link")
        .map(|(_, a)| {
            let get = |k: &str| {
                a.iter()
                    .find(|(x, _)| x == k)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            };
            (get("rel"), get("href"), get("type"))
        })
        .collect()
}

/// A local stand-in for openlibrary.org: every search finds one work with the requested title
/// by "Test <author>"; its ratings are derived from the title. Records each request.
pub struct FakeOpenLibrary {
    pub url: String,
    pub hits: std::sync::Arc<std::sync::Mutex<Vec<(String, std::time::Instant)>>>,
}

impl FakeOpenLibrary {
    pub fn count(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
}

/// Work number of a title (stable, digits only).
pub fn fake_work(title: &str) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for b in title.bytes() {
        h = (h ^ b as u64).wrapping_mul(1099511628211);
    }
    h % 1_000_000 + 1
}

pub async fn fake_openlibrary() -> FakeOpenLibrary {
    use axum::extract::{Path as UrlPath, Query, State};
    use std::collections::HashMap;
    type Hits = std::sync::Arc<std::sync::Mutex<Vec<(String, std::time::Instant)>>>;
    let hits: Hits = Default::default();
    async fn search(
        State(h): State<Hits>,
        Query(q): Query<HashMap<String, String>>,
    ) -> axum::Json<Value> {
        let title = q.get("title").cloned().unwrap_or_default();
        let author = q.get("author").cloned().unwrap_or_default();
        h.lock()
            .unwrap()
            .push((format!("search {title}"), std::time::Instant::now()));
        axum::Json(serde_json::json!({"numFound": 1, "docs": [{
            "key": format!("/works/OL{}W", fake_work(&title)),
            "title": title, "author_name": [format!("Test {author}")], "ratings_count": 5
        }]}))
    }
    async fn ratings(State(h): State<Hits>, UrlPath(id): UrlPath<String>) -> axum::Json<Value> {
        h.lock()
            .unwrap()
            .push((format!("ratings {id}"), std::time::Instant::now()));
        let n: u64 = id
            .trim_start_matches("OL")
            .trim_end_matches('W')
            .parse()
            .unwrap_or(1);
        axum::Json(serde_json::json!({"summary": {
            "average": 1.0 + (n % 40) as f64 / 10.0, "count": 1 + n % 97
        }}))
    }
    let app = axum::Router::new()
        .route("/search.json", axum::routing::get(search))
        .route("/works/{id}/ratings.json", axum::routing::get(ratings))
        .with_state(hits.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(l, app).await;
    });
    FakeOpenLibrary {
        url: format!("http://{addr}"),
        hits,
    }
}

/// Expected Open Library average of a fake work.
pub fn fake_avg(title: &str) -> f64 {
    1.0 + (fake_work(title) % 40) as f64 / 10.0
}
