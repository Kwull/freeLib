//! Start page, follows, editions and the search additions over HTTP.

mod common;

use common::{TestApp, make_library};
use serde_json::{Value, json};

async fn app() -> (TestApp, i64) {
    let app = TestApp::new(|_, root| {
        make_library(root, 300);
    })
    .await;
    let lib = app.import_library().await["id"].as_i64().unwrap();
    (app, lib)
}

/// A series whose first two books (in series order) share their first author.
async fn a_series(app: &TestApp, lib: i64) -> (i64, Vec<Value>) {
    let all = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5000"
        ))
        .await
        .json();
    let mut ids: Vec<i64> = all["books"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["series"]["id"].as_i64())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    for sid in ids {
        let books = app
            .get(&format!(
                "/api/v1/libraries/{lib}/books?series={sid}&group=1"
            ))
            .await
            .json()["books"]
            .as_array()
            .unwrap()
            .clone();
        if books.len() >= 2 && books[0]["authors"][0]["id"] == books[1]["authors"][0]["id"] {
            return (sid, books);
        }
    }
    panic!("no series with two books by one author");
}

#[tokio::test]
async fn start_page_follow_and_dismiss() {
    let (app, lib) = app().await;
    let home = format!("/api/v1/libraries/{lib}/home");

    // a new user: empty state with suggestions
    let h = app.get(&home).await.json();
    assert_eq!(h["empty"], true);
    assert!(!h["picks"].as_array().unwrap().is_empty());
    assert!(h["continueSeries"].as_array().unwrap().is_empty());
    assert_eq!(app.get(&format!("{home}?days=0")).await.status, 400);

    // download the first book of a series → "Continue series" offers the next one
    let (sid, books) = a_series(&app, lib).await;
    let first = books[0]["id"].as_i64().unwrap();
    let r = app
        .get(&format!("/api/v1/libraries/{lib}/books/{first}/file"))
        .await;
    assert_eq!(r.status, 200);
    let h = app.get(&format!("{home}?days=3650")).await.json();
    assert_eq!(h["empty"], false);
    let cont = h["continueSeries"].as_array().unwrap();
    let s = cont
        .iter()
        .find(|s| s["series"]["id"] == sid)
        .expect("series offered");
    assert_eq!(s["done"], 1);
    assert_eq!(s["next"][0]["id"], books[1]["id"]);
    assert!(s["next"][0]["rating"].is_number());
    // books by the author read are new (the one downloaded is not)
    let new = h["newFromAuthors"]["books"].as_array().unwrap();
    assert!(new.iter().all(|b| b["id"] != first));
    assert!(new.iter().any(|b| b["reason"]["kind"] == "author"));
    assert_eq!(h["newFromAuthors"]["days"], 3650);

    // dismiss / restore
    let r = app
        .post(&format!("{home}/dismiss"), &json!({"series": sid}))
        .await;
    assert_eq!(r.status, 204);
    let h = app.get(&home).await.json();
    assert!(
        h["continueSeries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["series"]["id"] != sid)
    );
    app.post(
        &format!("{home}/dismiss"),
        &json!({"series": sid, "dismissed": false}),
    )
    .await;
    let h = app.get(&home).await.json();
    assert!(
        h["continueSeries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["series"]["id"] == sid)
    );
    assert_eq!(
        app.post(&format!("{home}/dismiss"), &json!({"series": 999999}))
            .await
            .status,
        404
    );

    // follow an author and a series
    let follows = format!("/api/v1/libraries/{lib}/follows");
    let author = books[0]["authors"][0]["id"].as_i64().unwrap();
    let f = app
        .put(
            &follows,
            &json!({"kind": "author", "id": author, "follow": true}),
        )
        .await
        .json();
    assert_eq!(f["authors"][0]["id"], author);
    let f = app
        .put(
            &follows,
            &json!({"kind": "series", "id": sid, "follow": true}),
        )
        .await
        .json();
    assert_eq!(f["series"][0]["id"], sid);
    let h = app.get(&home).await.json();
    assert_eq!(h["following"], json!({"authors": 1, "series": 1}));
    let f = app
        .put(
            &follows,
            &json!({"kind": "author", "id": author, "follow": false}),
        )
        .await
        .json();
    assert!(f["authors"].as_array().unwrap().is_empty());
    assert_eq!(app.get(&follows).await.json()["series"][0]["id"], sid);
    assert_eq!(
        app.put(&follows, &json!({"kind": "genre", "id": 1, "follow": true}))
            .await
            .status,
        400
    );
    assert_eq!(
        app.put(
            &follows,
            &json!({"kind": "author", "id": 999999, "follow": true})
        )
        .await
        .status,
        404
    );
}

#[tokio::test]
async fn editions_and_search_fields() {
    let (app, lib) = app().await;
    // grouped lists never have more rows than books
    let all = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5000"
        ))
        .await
        .json();
    let grouped = app
        .get(&format!(
            "/api/v1/libraries/{lib}/books?since=1900-01-01&limit=5000&group=1"
        ))
        .await
        .json();
    assert!(grouped["total"].as_i64() <= all["total"].as_i64());
    let id = all["books"][0]["id"].as_i64().unwrap();
    let e = app
        .get(&format!("/api/v1/libraries/{lib}/books/{id}/editions"))
        .await
        .json();
    assert!(e["best"].is_number());
    let eds = e["books"].as_array().unwrap();
    assert!(eds.iter().any(|b| b["id"] == id));
    assert!(eds[0].get("note").is_some());
    assert_eq!(
        app.get(&format!("/api/v1/libraries/{lib}/books/999999/editions"))
            .await
            .status,
        404
    );
    // a grouped row lists its editions, the best copy first
    if let Some(b) = grouped["books"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["editions"].is_object())
    {
        assert!(b["editions"]["count"].as_i64().unwrap() >= 2);
        assert_eq!(b["editions"]["ids"][0], b["id"]);
    }

    // search: highlight, corrections
    let title = all["books"][0]["title"].as_str().unwrap().to_string();
    let word: String = title
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| w.chars().count() >= 5)
        .unwrap_or("книга")
        .to_string();
    let r = app
        .get(&format!(
            "/api/v1/libraries/{lib}/search?q={}&group=1",
            urlencode(&word)
        ))
        .await
        .json();
    assert!(r["highlight"].is_array());
    assert!(r.get("corrected").is_some() && r.get("didYouMean").is_some());
    // one letter dropped from a word the catalog knows → corrected
    let mut typo: Vec<char> = word.to_lowercase().chars().collect();
    typo.remove(typo.len() / 2);
    let typo: String = typo.into_iter().collect();
    let r = app
        .get(&format!(
            "/api/v1/libraries/{lib}/search?q={}",
            urlencode(&typo)
        ))
        .await
        .json();
    assert!(
        r["total"].as_i64().unwrap() > 0
            || r["corrected"].is_string()
            || r["didYouMean"].is_string(),
        "{typo}: {r}"
    );
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
