//! Ratings in the API: my / library / Open Library ratings and the kids age estimate in book
//! lists, server-side rating filters and sorts (books and search), the external rating cache,
//! and the background enrichment worker (priorities, request spacing, the admin switch).

mod common;

use std::time::{Duration, Instant};

use axum::http::StatusCode;
use common::{TestApp, fake_openlibrary, make_library};
use freelib_server::extrating::openlibrary::clean_title;
use serde_json::{Value, json};

async fn app_with_library(customize: impl FnOnce(&mut freelib_server::Config)) -> (TestApp, i64) {
    let app = TestApp::new(|cfg, root| {
        make_library(root, 300);
        customize(cfg);
    })
    .await;
    let lib = app.import_library().await;
    (app, lib["id"].as_i64().unwrap())
}

/// All pages of a book list.
async fn all_books(app: &TestApp, lib: i64, query: &str) -> (Vec<Value>, i64) {
    let mut out = Vec::new();
    let mut cursor = String::new();
    let mut total;
    loop {
        let r = app
            .get(&format!(
                "/api/v1/libraries/{lib}/books?{query}&limit=37{}",
                if cursor.is_empty() {
                    String::new()
                } else {
                    format!("&cursor={cursor}")
                }
            ))
            .await;
        assert_eq!(r.status, StatusCode::OK, "{query}: {}", r.text());
        let v = r.json();
        total = v["total"].as_i64().unwrap();
        out.extend(v["books"].as_array().unwrap().iter().cloned());
        match v["nextCursor"].as_str() {
            Some(c) => cursor = c.to_string(),
            None => break,
        }
    }
    assert_eq!(
        out.len() as i64,
        total,
        "{query}: pages add up to the total"
    );
    (out, total)
}

fn ids(v: &[Value]) -> Vec<i64> {
    v.iter().map(|b| b["id"].as_i64().unwrap()).collect()
}

fn non_increasing(v: &[f64]) -> bool {
    v.windows(2).all(|w| w[0] >= w[1])
}

#[tokio::test]
async fn rating_filters_and_sorts() {
    let (app, lib) = app_with_library(|_| {}).await;
    let (all, n) = all_books(&app, lib, "since=1900-01-01").await;
    assert!(n > 250);
    for b in &all {
        assert!(b["libRating"].as_i64().unwrap() <= 5, "{b}");
        assert!(
            b.get("kidsAge").is_some() && b.get("extRating").is_some(),
            "{b}"
        );
        let a = &b["kidsAge"];
        assert!(a.is_null() || [0, 6, 12, 16, 18].contains(&a.as_i64().unwrap()));
    }
    assert!(all.iter().any(|b| b["libRating"].as_i64().unwrap() > 0));
    assert!(all.iter().any(|b| !b["kidsAge"].is_null()));

    // my ratings: three books
    let rated = [
        (all[10]["id"].as_i64().unwrap(), 5),
        (all[20]["id"].as_i64().unwrap(), 3),
        (all[30]["id"].as_i64().unwrap(), 1),
    ];
    for (id, r) in rated {
        let resp = app
            .put(
                &format!("/api/v1/libraries/{lib}/books/{id}/rating"),
                &json!({"rating": r}),
            )
            .await;
        assert_eq!(resp.status, StatusCode::NO_CONTENT);
    }
    // sort=my: rated first (best first), everything else after, same total
    let (s, t) = all_books(&app, lib, "since=1900-01-01&sort=my").await;
    assert_eq!(t, n);
    assert_eq!(ids(&s[..3]), rated.map(|r| r.0).to_vec());
    assert!(s[3..].iter().all(|b| b["rating"] == 0));
    // minMy / unratedByMe
    let (s, _) = all_books(&app, lib, "since=1900-01-01&minMy=3").await;
    let mut got = ids(&s);
    got.sort();
    let mut want = vec![rated[0].0, rated[1].0];
    want.sort();
    assert_eq!(got, want);
    let (s, t) = all_books(&app, lib, "since=1900-01-01&unratedByMe=1").await;
    assert_eq!(t, n - 3);
    assert!(s.iter().all(|b| b["rating"] == 0));

    // library rating: filter and sort, on a genre scope paged from the server
    let want4 = all
        .iter()
        .filter(|b| b["libRating"].as_i64().unwrap() >= 4)
        .count() as i64;
    let (s, t) = all_books(&app, lib, "since=1900-01-01&minLib=4").await;
    assert_eq!(t, want4);
    assert!(s.iter().all(|b| b["libRating"].as_i64().unwrap() >= 4));
    let genres = app
        .get(&format!("/api/v1/libraries/{lib}/genres"))
        .await
        .json();
    let top = genres
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["parent"] == 0)
        .max_by_key(|g| g["count"].as_i64().unwrap())
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let (plain, pt) = all_books(&app, lib, &format!("genre={top}")).await;
    let (s, t) = all_books(&app, lib, &format!("genre={top}&sort=lib")).await;
    assert_eq!(t, pt);
    let mut a = ids(&plain);
    let mut b = ids(&s);
    a.sort();
    b.sort();
    assert_eq!(a, b, "same books, other order");
    let lr: Vec<f64> = s.iter().map(|b| b["libRating"].as_f64().unwrap()).collect();
    assert!(non_increasing(&lr), "{lr:?}");
    // equal ratings keep the list's own order (newest first)
    for w in s.windows(2) {
        if w[0]["libRating"] == w[1]["libRating"] {
            assert!(w[0]["date"].as_str() >= w[1]["date"].as_str());
        }
    }

    // kids age
    let want6 = all
        .iter()
        .filter(|b| b["kidsAge"].as_i64().is_some_and(|a| a <= 6))
        .count() as i64;
    let (s, t) = all_books(&app, lib, "since=1900-01-01&kidsMaxAge=6").await;
    assert_eq!(t, want6);
    assert!(s.iter().all(|b| b["kidsAge"].as_i64().unwrap() <= 6));

    // author and shelf scopes
    let author = all[10]["authors"][0]["id"].as_i64().unwrap();
    let (s, _) = all_books(&app, lib, &format!("author={author}&sort=my")).await;
    assert_eq!(s[0]["id"], rated[0].0);
    let shelf = app
        .post("/api/v1/shelves", &json!({"name": "S", "color": "#112233"}))
        .await
        .json();
    let sid = shelf["id"].as_i64().unwrap();
    let some: Vec<i64> = all
        .iter()
        .take(40)
        .map(|b| b["id"].as_i64().unwrap())
        .collect();
    app.post(
        &format!("/api/v1/shelves/{sid}/books"),
        &json!({"library": lib, "books": some, "add": true}),
    )
    .await;
    let (s, t) = all_books(&app, lib, &format!("shelf={sid}&sort=lib&minLib=1")).await;
    assert_eq!(
        t,
        all.iter()
            .take(40)
            .filter(|b| b["libRating"].as_i64().unwrap() >= 1)
            .count() as i64
    );
    let lr: Vec<f64> = s.iter().map(|b| b["libRating"].as_f64().unwrap()).collect();
    assert!(non_increasing(&lr));

    // search: sort and filters
    let word = all
        .iter()
        .flat_map(|b| {
            b["title"]
                .as_str()
                .unwrap()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.chars().count() >= 3)
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .fold(
            std::collections::HashMap::<String, usize>::new(),
            |mut m, w| {
                *m.entry(w).or_default() += 1;
                m
            },
        )
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .unwrap()
        .0;
    let r = app
        .get(&format!(
            "/api/v1/libraries/{lib}/search?q={word}&kind=books&sort=lib&limit=1000"
        ))
        .await
        .json();
    let lr: Vec<f64> = r["books"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["libRating"].as_f64().unwrap())
        .collect();
    assert!(lr.len() > 1 && non_increasing(&lr), "{word}: {lr:?}");
    let total = r["total"].as_i64().unwrap();
    let r = app
        .get(&format!(
            "/api/v1/libraries/{lib}/search?q={word}&kind=books&minLib=5&limit=1000"
        ))
        .await
        .json();
    assert!(r["total"].as_i64().unwrap() < total);
    assert!(
        r["books"]
            .as_array()
            .unwrap()
            .iter()
            .all(|b| b["libRating"] == 5)
    );
    let fl: i64 = r["facets"]["lang"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x[1].as_i64().unwrap())
        .sum();
    assert_eq!(
        fl,
        r["total"].as_i64().unwrap(),
        "facets count the filtered books"
    );

    // bad parameters
    for q in [
        "minMy=9",
        "minLib=x",
        "minExt=7",
        "minExtVotes=-1",
        "kidsMaxAge=old",
    ] {
        let r = app
            .get(&format!(
                "/api/v1/libraries/{lib}/books?since=1900-01-01&{q}"
            ))
            .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{q}");
    }
}

#[tokio::test]
async fn external_ratings_in_lists() {
    let ol = fake_openlibrary().await;
    let url = ol.url.clone();
    let (app, lib) = app_with_library(move |cfg| cfg.openlibrary_url = url).await;
    let (all, n) = all_books(&app, lib, "since=1900-01-01").await;
    // build the dense index first: later lookups must update it in place
    let (s, _) = all_books(&app, lib, "since=1900-01-01&sort=ext").await;
    assert!(s.iter().all(|b| b["extRating"].is_null()));
    let picked: Vec<i64> = all
        .iter()
        .filter(|b| {
            !b["authors"][0]["name"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("неизвест")
        })
        .step_by(29)
        .take(6)
        .map(|b| b["id"].as_i64().unwrap())
        .collect();
    for id in &picked {
        let e = app
            .state
            .ext
            .lookup_now(&app.state, lib, *id)
            .await
            .unwrap();
        assert_eq!(e.status, "found", "{id}");
    }
    let (s, t) = all_books(&app, lib, "since=1900-01-01&sort=ext").await;
    assert_eq!(t, n);
    let head: Vec<&Value> = s.iter().take(picked.len()).collect();
    let mut head_ids: Vec<i64> = head.iter().map(|b| b["id"].as_i64().unwrap()).collect();
    head_ids.sort();
    let mut want = picked.clone();
    want.sort();
    assert_eq!(head_ids, want, "rated books first");
    let avgs: Vec<f64> = head
        .iter()
        .map(|b| b["extRating"]["avg"].as_f64().unwrap())
        .collect();
    assert!(non_increasing(&avgs), "{avgs:?}");
    for b in &head {
        let title = clean_title(b["title"].as_str().unwrap());
        assert!((b["extRating"]["avg"].as_f64().unwrap() - common::fake_avg(&title)).abs() < 0.011);
        assert!(b["extRating"]["votes"].as_u64().unwrap() >= 1);
    }
    assert!(s[picked.len()..].iter().all(|b| b["extRating"].is_null()));
    // filters
    let min = avgs[avgs.len() / 2];
    let (s, _) = all_books(&app, lib, &format!("since=1900-01-01&minExt={min}")).await;
    assert_eq!(s.len(), avgs.iter().filter(|a| **a >= min - 1e-9).count());
    let (s, _) = all_books(&app, lib, "since=1900-01-01&minExtVotes=1").await;
    assert_eq!(s.len(), picked.len());
    let r = app
        .get(&format!(
            "/api/v1/libraries/{lib}/search?q={}&sort=ext",
            clean_title(
                all.iter().find(|b| b["id"] == picked[0]).unwrap()["title"]
                    .as_str()
                    .unwrap()
            )
            .split(' ')
            .next()
            .unwrap()
        ))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    // detail: the cached entry with a link
    let d = app
        .get(&format!("/api/v1/libraries/{lib}/books/{}", picked[0]))
        .await
        .json();
    assert_eq!(d["extRatingInfo"]["status"], "found");
    assert_eq!(d["extRatingInfo"]["source"], "openlibrary");
    assert!(
        d["extRatingInfo"]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://openlibrary.org/works/")
    );
    assert!(d["extRating"]["avg"].is_number());
    // progress: settings and the library
    let s = app.get("/api/v1/settings").await.json();
    assert_eq!(s["externalRatings"]["enabled"], true);
    assert_eq!(s["externalRatings"]["progress"]["lookedUp"], picked.len());
    assert_eq!(s["externalRatings"]["progress"]["found"], picked.len());
    assert_eq!(s["externalRatings"]["progress"]["total"], n);
    let libs = app.get("/api/v1/libraries").await.json();
    assert_eq!(libs[0]["externalRatings"]["lookedUp"], picked.len());
    // disabled: nothing is sent, on-demand lookups refuse
    app.put(
        "/api/v1/settings",
        &json!({"externalRatings": {"enabled": false}}),
    )
    .await;
    let before = ol.count();
    let other = all
        .iter()
        .map(|b| b["id"].as_i64().unwrap())
        .find(|id| !picked.contains(id))
        .unwrap();
    assert!(
        app.state
            .ext
            .lookup_now(&app.state, lib, other)
            .await
            .is_err()
    );
    // cached answers are still served
    assert!(
        app.state
            .ext
            .lookup_now(&app.state, lib, picked[0])
            .await
            .is_ok()
    );
    assert_eq!(ol.count(), before);
    // deleting the library forgets its ratings
    app.delete(&format!("/api/v1/libraries/{lib}")).await;
    assert_eq!(app.state.ext.progress(lib).looked_up, 0);
}

#[tokio::test]
async fn enrichment_worker_priorities_and_spacing() {
    let ol = fake_openlibrary().await;
    let url = ol.url.clone();
    let (app, lib) = app_with_library(move |cfg| {
        cfg.openlibrary_url = url;
        cfg.ext_worker = true;
        cfg.ext_interval = Duration::from_millis(60);
    })
    .await;
    // the slow sweep has started on its own
    let t0 = Instant::now();
    while ol.count() < 4 && t0.elapsed() < Duration::from_secs(20) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ol.count() >= 4, "the sweep sends requests");
    // browsing an author queues their books ahead of the sweep
    let (all, _) = all_books(&app, lib, "since=1900-01-01").await;
    let mut by_author: std::collections::HashMap<i64, Vec<&Value>> = Default::default();
    for b in &all {
        by_author
            .entry(b["authors"][0]["id"].as_i64().unwrap())
            .or_default()
            .push(b);
    }
    // an author whose books sit at the end of the library (the sweep goes by id)
    let (author, books) = by_author
        .iter()
        .filter(|(_, v)| v.len() >= 2 && v.len() <= 6)
        .filter(|(_, v)| {
            !v[0]["authors"][0]["name"]
                .as_str()
                .unwrap()
                .contains("неизвест")
        })
        .max_by_key(|(_, v)| v.iter().map(|b| b["id"].as_i64().unwrap()).min().unwrap())
        .unwrap();
    let _ = books;
    let mark = ol.count();
    let r = app
        .get(&format!("/api/v1/libraries/{lib}/books?author={author}"))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    // every book of the author scope (anthologies included) is queued
    let titles: std::collections::HashSet<String> = r.json()["books"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| {
            !b["authors"][0]["name"]
                .as_str()
                .unwrap()
                .contains("неизвест")
        })
        .map(|b| clean_title(b["title"].as_str().unwrap()))
        .collect();
    let t0 = Instant::now();
    loop {
        let (bs, _) = all_books(&app, lib, &format!("author={author}")).await;
        if bs.iter().all(|b| b["extRating"].is_object()) {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "author's books looked up"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let hits = ol.hits.lock().unwrap().clone();
    let after: Vec<String> = hits[mark..]
        .iter()
        .filter_map(|(u, _)| u.strip_prefix("search ").map(String::from))
        .collect();
    assert!(!after.is_empty());
    // at most one sweep item was in flight when the author's books were queued
    let first_n: Vec<&String> = after.iter().take(titles.len() + 1).collect();
    assert!(
        titles.iter().all(|t| first_n.contains(&t)),
        "{titles:?} first, got {first_n:?}"
    );
    // requests are at least the interval apart
    for w in hits.windows(2) {
        let gap = w[1].1.duration_since(w[0].1);
        assert!(gap >= Duration::from_millis(55), "gap {gap:?}");
    }
    // the admin switch stops the worker
    app.put(
        "/api/v1/settings",
        &json!({"externalRatings": {"enabled": false}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let n1 = ol.count();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(ol.count(), n1, "no requests while disabled");
    let s = app.get("/api/v1/settings").await.json();
    assert_eq!(s["externalRatings"]["enabled"], false);
    assert!(
        s["externalRatings"]["progress"]["lookedUp"]
            .as_u64()
            .unwrap()
            >= titles.len() as u64
    );
    assert!(s["externalRatings"]["requests"].as_u64().unwrap() >= 4);
}
