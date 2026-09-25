//! End-to-end API tests over a small generated library (docs/web/API.md).

mod common;

use std::io::Read;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use common::{TestApp, fake_calibre, links, make_library, parse_xml};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn app_with_library() -> (TestApp, i64) {
    let app = TestApp::new(|cfg, root| {
        make_library(root, 300);
        let _ = cfg;
    })
    .await;
    let lib = app.import_library().await;
    let id = lib["id"].as_i64().unwrap();
    (app, id)
}

/// First book (of the author/series list order) with `pred`.
async fn find_book(app: &TestApp, lib: i64, pred: impl Fn(&Value) -> bool) -> Value {
    let r = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5000"
        ))
        .await;
    r.json()["books"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| pred(b))
        .cloned()
        .expect("matching book")
}

fn zip_entry(path: &std::path::Path, name: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut e = z.by_name(name).unwrap();
    let mut v = Vec::new();
    e.read_to_end(&mut v).unwrap();
    v
}

#[tokio::test]
async fn open_mode_browse_and_download() {
    let (app, lib) = app_with_library().await;
    let s = app.get("/api/v1/session").await.json();
    assert_eq!(s["openMode"], true);
    assert_eq!(s["user"]["role"], "admin");

    let libs = app.get("/api/v1/libraries").await.json();
    let l = &libs[0];
    assert_eq!(l["name"], "Test");
    assert!(l["bookCount"].as_i64().unwrap() > 200);
    assert_eq!(l["status"]["state"], "idle");
    assert_eq!(l["opdsUrl"], format!("/opds/{lib}"));
    let version = l["catalogVersion"].as_i64().unwrap();
    assert!(version > 0);

    // authors: compact list, compressed, cacheable
    let r = app.get(&format!("/api/v1/libraries/{lib}/authors")).await;
    assert_eq!(r.status, StatusCode::OK);
    let a = r.json();
    assert_eq!(a["columns"], json!(["id", "name", "count"]));
    assert_eq!(a["version"], version);
    let rows = a["rows"].as_array().unwrap();
    assert!(rows.len() > 20);
    assert!(!a["letters"].as_array().unwrap().is_empty());
    assert_eq!(r.header("cache-control"), "private, no-cache");
    let etag = r.header("etag");
    let req = Request::builder()
        .uri(format!("/api/v1/libraries/{lib}/authors?v={version}"))
        .header(header::ACCEPT_ENCODING, "gzip, br")
        .body(Body::empty())
        .unwrap();
    let r2 = app.send(req).await;
    assert_eq!(r2.header("content-encoding"), "br");
    assert_eq!(
        r2.header("cache-control"),
        "public, max-age=31536000, immutable"
    );
    let mut dec = Vec::new();
    brotli::Decompressor::new(&r2.body[..], 4096)
        .read_to_end(&mut dec)
        .unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&dec).unwrap(), a);
    let req = Request::builder()
        .uri(format!("/api/v1/libraries/{lib}/authors"))
        .header(header::IF_NONE_MATCH, &etag)
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.send(req).await.status, StatusCode::NOT_MODIFIED);

    let series = app
        .get(&format!("/api/v1/libraries/{lib}/series"))
        .await
        .json();
    assert!(!series["rows"].as_array().unwrap().is_empty());
    let genres = app
        .get(&format!("/api/v1/libraries/{lib}/genres"))
        .await
        .json();
    assert!(genres.as_array().unwrap().len() > 300);

    // books by author / series / genre, pagination, errors
    let author_id = rows[0][0].as_i64().unwrap();
    let books = app
        .get(&format!("/api/v1/libraries/{lib}/books?author={author_id}"))
        .await
        .json();
    assert!(books["total"].as_i64().unwrap() >= 1);
    let b0 = &books["books"][0];
    assert!(
        b0["authors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == author_id)
    );
    assert_eq!(b0["rating"], 0);
    assert_eq!(b0["shelves"], json!([]));
    let sid = series["rows"][0][0].as_i64().unwrap();
    let sb = app
        .get(&format!("/api/v1/libraries/{lib}/books?series={sid}"))
        .await
        .json();
    assert!(
        sb["books"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["series"]["id"] == sid)
    );
    let p1 = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=10"
        ))
        .await
        .json();
    assert_eq!(p1["books"].as_array().unwrap().len(), 10);
    let cursor = p1["nextCursor"].as_str().unwrap();
    let p2 = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=10&cursor={cursor}"
        ))
        .await
        .json();
    assert_ne!(p1["books"][0]["id"], p2["books"][0]["id"]);
    let g = genres
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["count"].as_i64().unwrap() > 0 && g["parent"] != 0)
        .unwrap();
    let gb = app
        .get(&format!("/api/v1/libraries/{lib}/books?genre={}", g["id"]))
        .await
        .json();
    assert_eq!(gb["total"], g["count"]);
    assert_eq!(
        app.get(&format!("/api/v1/libraries/{lib}/books"))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.get(&format!("/api/v1/libraries/{lib}/books?author=1&series=1"))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.get(&format!("/api/v1/libraries/{lib}/books?author=1&cursor=zz"))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        app.get("/api/v1/libraries/99/books?author=1").await.status,
        StatusCode::NOT_FOUND
    );

    // detail with annotation and formats
    let fb2 = find_book(&app, lib, |b| b["ext"] == "fb2").await;
    let id = fb2["id"].as_i64().unwrap();
    let d = app
        .get(&format!("/api/v1/libraries/{lib}/books/{id}"))
        .await
        .json();
    assert!(d["annotation"].as_str().unwrap().contains("<p>"));
    assert_eq!(d["formats"], json!(["original", "epub", "kepub"]));
    assert!(d["file"].as_str().unwrap().contains(" / "));
    assert!(
        app.root()
            .join("cache/info")
            .join(lib.to_string())
            .read_dir()
            .unwrap()
            .count()
            >= 1
    );
    assert_eq!(
        app.get(&format!("/api/v1/libraries/{lib}/books/999999"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // original bytes equal the archive entry; EPUB and KEPUB conversions
    let orig = app
        .get(&format!("/api/v1/libraries/{lib}/books/{id}/file"))
        .await;
    assert_eq!(orig.status, StatusCode::OK);
    let dname = d["file"].as_str().unwrap().to_string();
    let (arch, entry) = dname.split_once(" / ").unwrap();
    assert_eq!(
        orig.body,
        zip_entry(&app.root().join("books/lib").join(arch), entry)
    );
    let cd = orig.header("content-disposition");
    assert!(
        cd.starts_with("attachment; filename=\"")
            && cd.contains("filename*=UTF-8''")
            && cd.ends_with(".fb2"),
        "{cd}"
    );
    let epub = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=epub"
        ))
        .await;
    assert_eq!(epub.status, StatusCode::OK, "{}", epub.text());
    assert_eq!(epub.header("content-type"), "application/epub+zip");
    assert_eq!(&epub.body[..2], b"PK");
    assert_eq!(&epub.body[30..38], b"mimetype");
    assert!(epub.header("content-disposition").ends_with(".epub"));
    let inline = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=epub&inline=1"
        ))
        .await;
    assert!(inline.header("content-disposition").starts_with("inline;"));
    assert_eq!(
        inline.body, epub.body,
        "second call served from the conversion cache"
    );
    assert!(
        app.root()
            .join("cache/out")
            .join(lib.to_string())
            .read_dir()
            .unwrap()
            .count()
            >= 1
    );
    let kepub = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=kepub"
        ))
        .await;
    assert_eq!(kepub.status, StatusCode::OK);
    assert!(kepub.header("content-disposition").ends_with(".kepub.epub"));
    let azw3 = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=azw3"
        ))
        .await;
    assert_eq!(azw3.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(azw3.json()["error"], "unsupported_format");
    assert_eq!(
        app.get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=docx"
        ))
        .await
        .status,
        StatusCode::BAD_REQUEST
    );

    // covers
    let mut with_cover = None;
    for b in app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=40"
        ))
        .await
        .json()["books"]
        .as_array()
        .unwrap()
    {
        let d = app
            .get(&format!("/api/v1/libraries/{lib}/books/{}", b["id"]))
            .await
            .json();
        if d["hasCover"] == true {
            with_cover = Some(b["id"].as_i64().unwrap());
            break;
        }
    }
    let cid = with_cover.expect("a book with a cover");
    let thumb = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{cid}/cover?size=thumb"
        ))
        .await;
    assert_eq!(thumb.status, StatusCode::OK);
    assert_eq!(thumb.header("content-type"), "image/webp");
    assert_eq!(&thumb.body[..4], b"RIFF");
    let full = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{cid}/cover?size=full"
        ))
        .await;
    assert_eq!(full.header("content-type"), "image/png");
    let req = Request::builder()
        .uri(format!(
            "/api/v1/libraries/{lib}/books/{cid}/cover?size=thumb"
        ))
        .header(header::IF_NONE_MATCH, thumb.header("etag"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.send(req).await.status, StatusCode::NOT_MODIFIED);
    // No real cover: `size=full` still 404s, but the default (`thumb`) gets a generated SVG
    // placeholder instead, with `X-Cover: generated`, so the client never has to 404-guess.
    let no_cover = find_book(&app, lib, |b| b["ext"] != "fb2" && b["ext"] != "epub").await;
    assert_eq!(
        app.get(&format!(
            "/api/v1/libraries/{lib}/books/{}/cover?size=full",
            no_cover["id"]
        ))
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    let placeholder = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{}/cover",
            no_cover["id"]
        ))
        .await;
    assert_eq!(placeholder.status, StatusCode::OK);
    assert_eq!(placeholder.header("content-type"), "image/svg+xml");
    assert_eq!(placeholder.header("x-cover"), "generated");
    assert!(placeholder.body.starts_with(b"<svg"));

    // search, languages
    let name = rows[0][1]
        .as_str()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    let q: String = name.chars().take(4).collect();
    let sr = app
        .get(&format!(
            "/api/v1/libraries/{lib}/search?q={}",
            urlencode(&q)
        ))
        .await;
    assert_eq!(sr.status, StatusCode::OK, "{}", sr.text());
    let sr = sr.json();
    assert!(!sr["authors"].as_array().unwrap().is_empty());
    assert!(sr["facets"]["lang"].is_array());
    assert!(sr["tookMs"].is_number());
    assert_eq!(
        app.get(&format!("/api/v1/libraries/{lib}/search?q=a"))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let langs = app
        .get(&format!("/api/v1/languages?lib={lib}"))
        .await
        .json();
    assert!(langs[0][0].is_string() && langs[0][1].is_number());

    // rating
    let r = app
        .put(
            &format!("/api/v1/libraries/{lib}/books/{id}/rating"),
            &json!({"rating": 4}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.get(&format!("/api/v1/libraries/{lib}/books/{id}"))
            .await
            .json()["rating"],
        4
    );
    assert_eq!(
        app.put(
            &format!("/api/v1/libraries/{lib}/books/{id}/rating"),
            &json!({"rating": 9})
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );

    // prefs
    let p = app
        .put(
            "/api/v1/me/prefs",
            &json!({"currentLibrary": lib, "view": "grid"}),
        )
        .await;
    assert_eq!(p.status, StatusCode::OK);
    assert_eq!(app.get("/api/v1/me/prefs").await.json()["view"], "grid");
    let big = json!({"x": "y".repeat(70_000)});
    assert_eq!(
        app.put("/api/v1/me/prefs", &big).await.status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(app.get("/api/v1/fonts").await.json(), json!(["PT Serif"]));

    // library update / re-import conflict / delete
    let r = app
        .patch(
            &format!("/api/v1/libraries/{lib}"),
            &json!({"name": "Renamed"}),
        )
        .await;
    assert_eq!(r.json()["name"], "Renamed");
    let job = app
        .post(
            &format!("/api/v1/libraries/{lib}/import"),
            &json!({"mode": "new"}),
        )
        .await;
    assert_eq!(job.status, StatusCode::OK);
    let again = app
        .post(
            &format!("/api/v1/libraries/{lib}/import"),
            &json!({"mode": "full"}),
        )
        .await;
    assert!(again.status == StatusCode::CONFLICT || again.status == StatusCode::OK);
    let done = app.wait_job(job.json()["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done");
    if again.status == StatusCode::OK {
        app.wait_job(again.json()["id"].as_str().unwrap()).await;
    }
    let l2 = app.get("/api/v1/libraries").await.json();
    assert!(l2[0]["catalogVersion"].as_i64().unwrap() > version);
    // user data keyed by book_key survives the re-import
    assert_eq!(
        app.get(&format!("/api/v1/libraries/{lib}/books/{id}"))
            .await
            .json()["rating"],
        4
    );
    assert_eq!(
        app.delete(&format!("/api/v1/libraries/{lib}")).await.status,
        StatusCode::NO_CONTENT
    );
    assert!(!app.root().join(format!("data/lib_{lib}.db")).exists());
    assert_eq!(app.get("/api/v1/libraries").await.json(), json!([]));
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[tokio::test]
async fn shelves_devices_and_jobs() {
    let (app, lib) = app_with_library().await;
    let books = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5"
        ))
        .await
        .json();
    let ids: Vec<i64> = books["books"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_i64().unwrap())
        .collect();

    // shelves
    let s = app
        .post(
            "/api/v1/shelves",
            &json!({"name": "To read", "color": "#1F5F5B"}),
        )
        .await
        .json();
    let sid = s["id"].as_i64().unwrap();
    assert_eq!(s["count"], 0);
    assert_eq!(
        app.post("/api/v1/shelves", &json!({"name": "x", "color": "red"}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let s2 = app
        .post(
            &format!("/api/v1/shelves/{sid}/books"),
            &json!({"library": lib, "books": ids, "add": true}),
        )
        .await
        .json();
    assert_eq!(s2["count"], 5);
    let sb = app
        .get(&format!("/api/v1/libraries/{lib}/books?shelf={sid}"))
        .await
        .json();
    assert_eq!(sb["total"], 5);
    assert!(
        sb["books"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["shelves"] == json!([sid]))
    );
    let s3 = app
        .post(
            &format!("/api/v1/shelves/{sid}/books"),
            &json!({"library": lib, "books": [ids[0]], "add": false}),
        )
        .await
        .json();
    assert_eq!(s3["count"], 4);
    let s4 = app
        .patch(&format!("/api/v1/shelves/{sid}"), &json!({"name": "Later"}))
        .await
        .json();
    assert_eq!(s4["name"], "Later");
    assert_eq!(
        app.get("/api/v1/shelves")
            .await
            .json()
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // devices: shared defaults + own
    let devs = app.get("/api/v1/devices").await.json();
    let names: Vec<&str> = devs
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Kindle",
            "Kindle (USB)",
            "Apple Books",
            "Kobo",
            "Server folder",
            "Original"
        ]
    );
    assert!(devs.as_array().unwrap().iter().all(|d| d["shared"] == true));
    let dev = app
        .post(
            "/api/v1/devices",
            &json!({"name": "My Kobo", "kind": "download", "format": "kepub", "target": null, "fileName": "%a/%s %n %b",
                    "shared": false, "options": {"hyphenate": "soft", "footnotes": "popup"}}),
        )
        .await;
    assert_eq!(dev.status, StatusCode::OK, "{}", dev.text());
    let dev = dev.json();
    assert_eq!(dev["options"]["hyphenate"], "soft");
    assert_eq!(dev["options"]["breakAfterChapter"], true);
    let mut upd = dev.clone();
    upd["name"] = json!("Kobo Libra");
    assert_eq!(
        app.put(&format!("/api/v1/devices/{}", dev["id"]), &upd)
            .await
            .json()["name"],
        "Kobo Libra"
    );
    let bad = app
        .post(
            "/api/v1/devices",
            &json!({"name": "x", "kind": "folder", "format": "epub", "target": "../../etc"}),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);

    // SSE subscriber
    let req = Request::builder()
        .uri("/api/v1/events")
        .body(Body::empty())
        .unwrap();
    let resp = app.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.headers()["content-type"], "text/event-stream");
    let mut body = resp.into_body();
    let sse = tokio::spawn(async move {
        let mut text = String::new();
        while let Ok(Some(Ok(frame))) =
            tokio::time::timeout(Duration::from_secs(20), body.frame()).await
        {
            if let Some(d) = frame.data_ref() {
                text.push_str(&String::from_utf8_lossy(d));
            }
            if text.contains("event: job") && text.contains("\"state\":\"done\"") {
                break;
            }
        }
        text
    });

    // download of several books → zip
    let original = devs
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "Original")
        .unwrap();
    let job = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": &ids[..3], "device": original["id"]}),
        )
        .await;
    assert_eq!(job.status, StatusCode::OK, "{}", job.text());
    let job = job.json();
    assert_eq!(job["kind"], "download");
    let done = app.wait_job(job["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    let url = done["downloadUrl"].as_str().unwrap();
    let z = app.get(url).await;
    assert_eq!(z.status, StatusCode::OK);
    assert_eq!(z.header("content-type"), "application/zip");
    let za = zip::ZipArchive::new(std::io::Cursor::new(z.body.clone())).unwrap();
    assert_eq!(za.len(), 3);
    let events = sse.await.unwrap();
    assert!(events.contains("event: job"), "{events}");
    assert!(events.contains(job["id"].as_str().unwrap()));

    // single-book EPUB download → the file itself
    let apple = devs
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "Apple Books")
        .unwrap();
    let fb2 = find_book(&app, lib, |b| b["ext"] == "fb2").await;
    let job = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": [fb2["id"]], "device": apple["id"]}),
        )
        .await
        .json();
    let done = app.wait_job(job["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    let f = app.get(done["downloadUrl"].as_str().unwrap()).await;
    assert_eq!(f.header("content-type"), "application/epub+zip");
    assert!(f.header("content-disposition").contains(".epub"));

    // joined series
    let series_rows = app
        .get(&format!("/api/v1/libraries/{lib}/series"))
        .await
        .json();
    let big = series_rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r[2].as_i64().unwrap() >= 2);
    if let Some(sr) = big {
        let sb = app
            .get(&format!("/api/v1/libraries/{lib}/books?series={}", sr[0]))
            .await
            .json();
        let fb2s: Vec<i64> = sb["books"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|b| b["ext"] == "fb2")
            .map(|b| b["id"].as_i64().unwrap())
            .collect();
        if fb2s.len() >= 2 {
            let d = app
                .post(
                    "/api/v1/devices",
                    &json!({"name": "Joined", "kind": "download", "format": "epub", "fileName": "%s", "options": {"joinSeries": true}}),
                )
                .await
                .json();
            let job = app
                .post(
                    "/api/v1/send",
                    &json!({"library": lib, "books": fb2s, "device": d["id"]}),
                )
                .await
                .json();
            let done = app.wait_job(job["id"].as_str().unwrap()).await;
            assert_eq!(done["state"], "done", "{done}");
            let f = app.get(done["downloadUrl"].as_str().unwrap()).await;
            assert_eq!(
                f.header("content-type"),
                "application/epub+zip",
                "one joined file"
            );
        }
    }

    // export to the server folder
    let folder = devs
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "Server folder")
        .unwrap();
    let job = app.post("/api/v1/send", &json!({"library": lib, "books": &ids[..2], "device": folder["id"], "target": "incoming"})).await.json();
    assert_eq!(job["kind"], "export");
    let done = app.wait_job(job["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    let exported: Vec<_> = std::fs::read_dir(app.root().join("export/incoming"))
        .unwrap()
        .collect();
    assert_eq!(exported.len(), 2);
    let bad = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": &ids[..1], "device": folder["id"], "target": "../x"}),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);

    // email without SMTP settings → 400; azw3 without Calibre → 501
    let kindle = devs
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "Kindle")
        .unwrap();
    let r = app.post("/api/v1/send", &json!({"library": lib, "books": &ids[..1], "device": kindle["id"], "target": "me@kindle.com"})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let usb = devs
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "Kindle (USB)")
        .unwrap();
    let r = app
        .post(
            "/api/v1/send",
            &json!({"library": lib, "books": &ids[..1], "device": usb["id"]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_IMPLEMENTED);

    // jobs: list, clear finished
    let jobs = app.get("/api/v1/jobs").await.json();
    assert!(jobs.as_array().unwrap().len() >= 3);
    assert_eq!(
        app.delete("/api/v1/jobs?finished=1").await.status,
        StatusCode::NO_CONTENT
    );
    assert!(
        app.get("/api/v1/jobs")
            .await
            .json()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(app.get(url).await.status, StatusCode::NOT_FOUND);

    assert_eq!(
        app.delete(&format!("/api/v1/shelves/{sid}")).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.delete(&format!("/api/v1/devices/{}", dev["id"]))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn fs_and_request_security() {
    let app = TestApp::new(|cfg, root| {
        cfg.allowed_hosts = vec!["books.example".into()];
        make_library(root, 50);
        std::fs::create_dir_all(root.join("outside")).unwrap();
        std::os::unix::fs::symlink(root.join("outside"), root.join("books/escape")).unwrap();
    })
    .await;
    let r = app.get("/api/v1/fs").await.json();
    assert_eq!(r["path"], "");
    assert_eq!(r["parent"], Value::Null);
    let names: Vec<&str> = r["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["lib"],
        "symlink escaping the books folder is hidden"
    );
    let r = app.get("/api/v1/fs?path=lib").await.json();
    assert_eq!(r["parent"], "");
    assert!(
        r["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "test.inpx" && e["dir"] == false)
    );
    assert!(
        r["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"].as_str().unwrap().ends_with(".zip"))
    );
    for bad in ["..", "lib/../..", "/etc", "escape", "escape/..", "%2e%2e"] {
        let r = app.get(&format!("/api/v1/fs?path={bad}")).await;
        assert!(
            r.status == StatusCode::FORBIDDEN || r.status == StatusCode::BAD_REQUEST,
            "{bad}: {}",
            r.status
        );
    }
    let r = app
        .post("/api/v1/libraries", &json!({"name": "x", "path": "/tmp"}))
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = app
        .post("/api/v1/libraries", &json!({"name": "x", "path": "escape"}))
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // CSRF: non-JSON bodies, foreign origins and cross-site fetches are refused
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/shelves")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("{\"name\":\"a\",\"color\":\"#000000\"}"))
        .unwrap();
    assert_eq!(
        app.send(req).await.status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let mk = |origin: &str, site: Option<&str>| {
        let mut b = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/shelves")
            .header(header::HOST, "books.example:8080")
            .header(header::ORIGIN, origin)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(s) = site {
            b = b.header("sec-fetch-site", s);
        }
        b.body(Body::from("{\"name\":\"a\",\"color\":\"#000000\"}"))
            .unwrap()
    };
    assert_eq!(
        app.send(mk("http://evil.example", None)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(mk("http://books.example:8080", Some("cross-site")))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(mk("http://books.example:8080", None)).await.status,
        StatusCode::OK
    );
    assert_eq!(
        app.send(mk("http://localhost:5173", Some("same-origin")))
            .await
            .status,
        StatusCode::OK
    );
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/logout")
        .header(header::ORIGIN, "http://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.send(req).await.status, StatusCode::FORBIDDEN);
    // body limit
    let huge = json!({"name": "x".repeat(2_000_000), "color": "#000000"});
    assert_eq!(
        app.post("/api/v1/shelves", &huge).await.status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    // unknown API path → JSON 404, SPA elsewhere
    let r = app.get("/api/v1/nope").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.json()["error"], "not_found");
}

#[tokio::test]
async fn login_mode_and_permissions() {
    let mut app = TestApp::new(|cfg, root| {
        make_library(root, 50);
        cfg.admin_password = Some("secret-pw".into());
    })
    .await;
    let s = app.get("/api/v1/session").await.json();
    assert_eq!(s, json!({"user": null, "openMode": false}));
    assert_eq!(
        app.get("/api/v1/libraries").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.get("/api/v1/libraries").await.json()["error"],
        "unauthorized"
    );
    assert_eq!(
        app.login("admin", "wrong").await.status,
        StatusCode::UNAUTHORIZED
    );
    let r = app.login("admin", "secret-pw").await;
    assert_eq!(r.status, StatusCode::OK);
    let cookie = r.header("set-cookie");
    assert!(
        cookie.starts_with("freelib_session=")
            && cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Lax")
    );
    assert!(cookie.contains("Max-Age=2592000"));
    assert_eq!(r.json()["user"]["role"], "admin");
    assert_eq!(
        app.get("/api/v1/session").await.json()["user"]["username"],
        "admin"
    );
    let lib = app.import_library().await["id"].as_i64().unwrap();
    let admin_cookie = app.cookie.clone();

    // a reader
    let u = app
        .post(
            "/api/v1/users",
            &json!({"username": "reader", "password": "readpw", "role": "reader"}),
        )
        .await;
    assert_eq!(u.status, StatusCode::OK, "{}", u.text());
    let reader_id = u.json()["id"].as_i64().unwrap();
    assert_eq!(
        app.post(
            "/api/v1/users",
            &json!({"username": "reader", "password": "readpw"})
        )
        .await
        .status,
        StatusCode::CONFLICT
    );
    let admin_shelf = app
        .post(
            "/api/v1/shelves",
            &json!({"name": "Admin shelf", "color": "#000000"}),
        )
        .await
        .json();
    let settings = app.get("/api/v1/settings").await.json();
    assert_eq!(settings["opds"]["requireAuth"], true);

    app.cookie = None;
    assert_eq!(app.login("reader", "readpw").await.status, StatusCode::OK);
    assert_eq!(
        app.get("/api/v1/settings").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(app.get("/api/v1/users").await.status, StatusCode::FORBIDDEN);
    assert_eq!(app.get("/api/v1/fs").await.status, StatusCode::FORBIDDEN);
    assert_eq!(
        app.post(
            &format!("/api/v1/libraries/{lib}/import"),
            &json!({"mode": "full"})
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.get("/api/v1/libraries")
            .await
            .json()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        app.get("/api/v1/shelves")
            .await
            .json()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let r = app
        .post(
            &format!("/api/v1/shelves/{}/books", admin_shelf["id"]),
            &json!({"library": lib, "books": [1], "add": true}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(
        app.get(&format!(
            "/api/v1/libraries/{lib}/books?shelf={}",
            admin_shelf["id"]
        ))
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert!(
        app.get("/api/v1/jobs")
            .await
            .json()
            .as_array()
            .unwrap()
            .is_empty(),
        "imports are admin-only"
    );
    let r = app
        .post(
            "/api/v1/devices",
            &json!({"name": "S", "kind": "download", "format": "epub", "shared": true}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let devs = app.get("/api/v1/devices").await.json();
    let shared = &devs[0];
    assert_eq!(
        app.put(&format!("/api/v1/devices/{}", shared["id"]), shared)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.delete(&format!("/api/v1/devices/{}", shared["id"]))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.put(
            &format!("/api/v1/libraries/{lib}/books/1/rating"),
            &json!({"rating": 5})
        )
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.post_empty("/api/v1/logout").await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.get("/api/v1/shelves").await.status,
        StatusCode::UNAUTHORIZED
    );

    // ratings are per user
    app.cookie = admin_cookie;
    assert_eq!(
        app.get(&format!("/api/v1/libraries/{lib}/books/1"))
            .await
            .json()["rating"],
        0
    );
    // last admin cannot be deleted or demoted; users can be removed
    let me = app.get("/api/v1/session").await.json()["user"]["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        app.delete(&format!("/api/v1/users/{me}")).await.status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.patch(&format!("/api/v1/users/{me}"), &json!({"role": "reader"}))
            .await
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.patch(
            &format!("/api/v1/users/{reader_id}"),
            &json!({"password": "newpass"})
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        app.delete(&format!("/api/v1/users/{reader_id}"))
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.get("/api/v1/users")
            .await
            .json()
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // OPDS requires auth (Basic)
    let r = app
        .send(Request::builder().uri("/opds").body(Body::empty()).unwrap())
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(r.header("www-authenticate").starts_with("Basic"));
    use base64::Engine;
    let basic = base64::engine::general_purpose::STANDARD.encode("admin:secret-pw");
    let r = app
        .send(
            Request::builder()
                .uri("/opds")
                .header(header::AUTHORIZATION, format!("Basic {basic}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);

    // login rate limiting
    app.cookie = None;
    for _ in 0..5 {
        assert_eq!(
            app.login("admin", "nope").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let r = app.login("admin", "secret-pw").await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.json()["error"], "rate_limited");
}

#[tokio::test]
async fn opds_feeds() {
    let (app, lib) = app_with_library().await;
    let nav = "application/atom+xml;profile=opds-catalog;kind=navigation";
    let acq = "application/atom+xml;profile=opds-catalog;kind=acquisition";
    let root = app.get("/opds").await;
    assert_eq!(root.status, StatusCode::OK);
    assert!(
        root.header("content-type")
            .starts_with("application/atom+xml")
    );
    let l = links(&root.body);
    assert!(l.contains(&("self".into(), "/opds".into(), nav.into())));
    assert!(l.contains(&(
        "search".into(),
        format!("/opds/{lib}/opensearch.xml"),
        "application/opensearchdescription+xml".into()
    )));
    assert!(l.contains(&(
        "subsection".into(),
        format!("/opds/{lib}/authors"),
        nav.into()
    )));
    assert!(l.iter().any(|x| x.0 == "http://opds-spec.org/sort/new"));

    let osd = app.get(&format!("/opds/{lib}/opensearch.xml")).await;
    let els = parse_xml(&osd.body);
    assert!(els.iter().any(|(n, a)| {
        n == "Url"
            && a.iter()
                .any(|(k, v)| k == "template" && v.contains("{searchTerms}"))
    }));

    let authors = app.get(&format!("/opds/{lib}/authors")).await;
    let letters = links(&authors.body);
    let first = letters
        .iter()
        .find(|x| x.0 == "subsection")
        .expect("letter entries");
    let letter_feed = app.get(&first.1).await;
    assert_eq!(letter_feed.status, StatusCode::OK, "{}", first.1);
    let author_link = links(&letter_feed.body)
        .into_iter()
        .find(|x| x.1.contains("/author/"))
        .expect("author entries under a letter");
    assert_eq!(author_link.2, acq);
    let books = app.get(&author_link.1).await;
    let bl = links(&books.body);
    let acqs: Vec<_> = bl
        .iter()
        .filter(|x| x.0 == "http://opds-spec.org/acquisition/open-access")
        .collect();
    assert!(!acqs.is_empty());
    assert!(acqs.iter().any(|x| x.2 == "application/epub+zip"));
    let els = parse_xml(&books.body);
    assert!(els.iter().any(|(n, _)| n == "entry"));
    assert!(els.iter().any(|(n, _)| n == "author"));

    // download through OPDS
    let epub_link = acqs.iter().find(|x| x.1.ends_with("/epub")).unwrap();
    let f = app.get(&epub_link.1).await;
    assert_eq!(f.status, StatusCode::OK);
    assert_eq!(&f.body[..2], b"PK");

    for path in ["series", "genres", "new", "search?q=%D0%B0%D0%BB"] {
        let r = app.get(&format!("/opds/{lib}/{path}")).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        parse_xml(&r.body);
    }
    let genres = app.get(&format!("/opds/{lib}/genres")).await;
    let g = links(&genres.body)
        .into_iter()
        .find(|x| x.0 == "subsection")
        .unwrap();
    let gr = app.get(&g.1).await;
    assert_eq!(gr.status, StatusCode::OK);
    parse_xml(&gr.body);
    let series = app.get(&format!("/opds/{lib}/series")).await;
    let s = links(&series.body)
        .into_iter()
        .find(|x| x.0 == "subsection")
        .unwrap();
    let sr = app.get(&s.1).await;
    assert_eq!(sr.status, StatusCode::OK);
    parse_xml(&sr.body);

    // legacy Qt paths
    for (old, new) in [
        (format!("/opds_{lib}"), format!("/opds/{lib}")),
        (
            format!("/opds_{lib}/authorsindex"),
            format!("/opds/{lib}/authors"),
        ),
        (
            format!("/opds_{lib}/sequencesindex"),
            format!("/opds/{lib}/series"),
        ),
        (format!("/opds_{lib}/genres"), format!("/opds/{lib}/genres")),
        (
            format!("/opds_{lib}/search?search_string=abc"),
            format!("/opds/{lib}/search?q=abc"),
        ),
    ] {
        let r = app.get(&old).await;
        assert_eq!(r.status, StatusCode::MOVED_PERMANENTLY, "{old}");
        assert_eq!(r.header("location"), new);
    }
    assert_eq!(app.get("/opds/999").await.status, StatusCode::NOT_FOUND);
}

/// A tiny SMTP server that records received messages.
async fn fake_smtp() -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let msgs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let m2 = msgs.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else {
                break;
            };
            let msgs = m2.clone();
            tokio::spawn(async move {
                let (r, mut w) = sock.into_split();
                let mut lines = BufReader::new(r).lines();
                w.write_all(b"220 fake ESMTP\r\n").await.unwrap();
                let mut data: Option<String> = None;
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(d) = data.as_mut() {
                        if line == "." {
                            msgs.lock().unwrap().push(data.take().unwrap());
                            w.write_all(b"250 OK queued\r\n").await.unwrap();
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
                        w.write_all(b"221 bye\r\n").await.unwrap();
                        break;
                    } else {
                        b"250 OK\r\n"
                    };
                    w.write_all(reply).await.unwrap();
                }
            });
        }
    });
    (port, msgs)
}

#[tokio::test]
async fn calibre_and_smtp() {
    let app = TestApp::new(|cfg, root| {
        make_library(root, 100);
        cfg.calibre = Some(fake_calibre(root));
    })
    .await;
    let lib = app.import_library().await["id"].as_i64().unwrap();
    let s = app.get("/api/v1/settings").await.json();
    assert_eq!(s["calibre"], json!({"available": true, "version": "7.4.0"}));

    let fb2 = find_book(&app, lib, |b| b["ext"] == "fb2").await;
    let id = fb2["id"].clone();
    let d = app
        .get(&format!("/api/v1/libraries/{lib}/books/{id}"))
        .await
        .json();
    assert_eq!(
        d["formats"],
        json!(["original", "epub", "kepub", "azw3", "mobi", "pdf"])
    );
    let epub = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=epub"
        ))
        .await;
    let azw3 = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{id}/file?format=azw3"
        ))
        .await;
    assert_eq!(azw3.status, StatusCode::OK, "{}", azw3.text());
    assert_eq!(azw3.body, epub.body, "fake Calibre copies the EPUB");
    assert!(azw3.header("content-disposition").ends_with(".azw3"));
    let txt = find_book(&app, lib, |b| b["ext"] != "fb2" && b["ext"] != "epub").await;
    let ext = txt["ext"].as_str().unwrap();
    let f = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books/{}/file?format=pdf",
            txt["id"]
        ))
        .await;
    // Calibre only ever gets EPUB input: other originals are not converted
    assert_eq!(f.status, StatusCode::NOT_IMPLEMENTED, "{ext}");

    // SMTP: settings (password write-only), test mail, Send to Kindle job
    let (port, msgs) = fake_smtp().await;
    let r = app
        .put(
            "/api/v1/settings",
            &json!({"smtp": {"host": "127.0.0.1", "port": port, "security": "none", "username": "", "from": "lib@example.com",
                             "password": "smtp-secret", "pauseSeconds": 0},
                    "opds": {"enabled": true, "requireAuth": false}}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let s = r.json();
    assert_eq!(s["smtp"]["passwordSet"], true);
    assert!(!r.text().contains("smtp-secret"));
    let s2 = app
        .put("/api/v1/settings", &json!({"smtp": {"host": "127.0.0.1"}}))
        .await
        .json();
    assert_eq!(s2["smtp"]["passwordSet"], true, "omitted password is kept");
    let t = app
        .post(
            "/api/v1/settings/smtp/test",
            &json!({"to": "me@example.com"}),
        )
        .await;
    assert_eq!(t.status, StatusCode::NO_CONTENT, "{}", t.text());
    assert_eq!(msgs.lock().unwrap().len(), 1);

    let devs = app.get("/api/v1/devices").await.json();
    let kindle = devs
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["name"] == "Kindle")
        .unwrap();
    let job = app.post("/api/v1/send", &json!({"library": lib, "books": [id], "device": kindle["id"], "target": "me@kindle.com"})).await;
    assert_eq!(job.status, StatusCode::OK, "{}", job.text());
    let job = job.json();
    assert_eq!(job["kind"], "send");
    let done = app.wait_job(job["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "done", "{done}");
    let m = msgs.lock().unwrap();
    assert_eq!(m.len(), 2);
    assert!(m[1].contains("me@kindle.com"));
    assert!(m[1].contains("application/epub+zip"));
    assert!(m[1].contains(".epub"));
}

#[tokio::test]
async fn spa_serving() {
    let app = TestApp::new(|cfg, root| {
        let web = root.join("web");
        std::fs::create_dir_all(web.join("assets")).unwrap();
        std::fs::write(
            web.join("index.html"),
            "<!doctype html><title>freeLib</title>",
        )
        .unwrap();
        std::fs::write(web.join("assets/app-123.js"), "console.log(1)").unwrap();
        cfg.web_dir = Some(web);
    })
    .await;
    for p in ["/", "/l/1/authors/5", "/settings/mail"] {
        let r = app.get(p).await;
        assert_eq!(r.status, StatusCode::OK, "{p}");
        assert!(r.text().contains("<title>freeLib</title>"));
        assert_eq!(r.header("cache-control"), "no-cache");
    }
    let js = app.get("/assets/app-123.js").await;
    assert_eq!(
        js.header("cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert!(js.header("content-type").contains("javascript"));
    assert_eq!(
        app.get("/assets/missing.js").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get("/assets/../../etc/passwd").await.status,
        StatusCode::NOT_FOUND
    );
}
