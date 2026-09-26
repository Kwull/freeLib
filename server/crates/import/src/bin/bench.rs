//! Import + query benchmark on a synthetic (or real) INPX.
//!
//! ```text
//! cargo run --release -p freelib-import --bin bench -- [--books 600000] [--inpx FILE] [--db FILE] [--lib-dir DIR] [--skip-import]
//! ```
//! Generates `bench-data/synthetic-<N>.inpx` when missing, imports it into `bench-data/bench_lib.db`
//! and prints timings of the queries behind the ARCHITECTURE.md performance targets.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use freelib_catalog::{BookFilter, BookSelector, Catalog, Page, SearchKind, SearchQuery};
use freelib_import::synth::{GenOptions, Rng, generate};
use freelib_import::{ImportOptions, import_inpx};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

struct Series {
    name: String,
    samples: Vec<f64>,
}

impl Series {
    fn new(name: &str) -> Self {
        Series {
            name: name.into(),
            samples: vec![],
        }
    }
    fn time<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let r = f();
        self.samples.push(ms(t.elapsed()));
        r
    }
    fn report(&mut self) {
        self.samples.sort_by(|a, b| a.total_cmp(b));
        let n = self.samples.len();
        if n == 0 {
            return;
        }
        let p = |q: f64| self.samples[((n as f64 * q).ceil() as usize).clamp(1, n) - 1];
        println!(
            "| {:<44} | {:>5} | {:>8.2} | {:>8.2} | {:>8.2} |",
            self.name,
            n,
            p(0.5),
            p(0.95),
            self.samples[n - 1]
        );
    }
}

fn main() {
    let mut books = 600_000usize;
    let mut inpx: Option<PathBuf> = None;
    let mut db = PathBuf::from("bench-data/bench_lib.db");
    let mut skip_import = false;
    let mut lib_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--books" => books = args.next().and_then(|v| v.parse().ok()).expect("--books N"),
            "--inpx" => inpx = args.next().map(PathBuf::from),
            "--db" => db = args.next().map(PathBuf::from).expect("--db FILE"),
            "--skip-import" => skip_import = true,
            "--lib-dir" => lib_dir = args.next().map(PathBuf::from),
            _ => {
                eprintln!(
                    "usage: bench [--books N] [--inpx FILE] [--db FILE] [--lib-dir DIR] [--skip-import]"
                );
                std::process::exit(2);
            }
        }
    }
    let inpx = inpx
        .unwrap_or_else(|| PathBuf::from(format!("bench-data/synthetic-{}k.inpx", books / 1000)));
    if !inpx.exists() {
        std::fs::create_dir_all(inpx.parent().unwrap()).unwrap();
        let t = Instant::now();
        generate(
            &inpx,
            &GenOptions {
                books,
                ..Default::default()
            },
        )
        .expect("generate");
        println!(
            "generated {} in {:.1}s",
            inpx.display(),
            t.elapsed().as_secs_f64()
        );
    }

    if !skip_import {
        let opts = ImportOptions {
            inpx: inpx.clone(),
            db_path: db.clone(),
            resolve_offsets: lib_dir.is_some(),
            library_dir: lib_dir.clone(),
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let t = Instant::now();
        let stats = import_inpx(
            &opts,
            &|done, total, msg| {
                if done % 100 == 0 || done >= total.saturating_sub(5) {
                    eprintln!("  [{done}/{total}] {msg}");
                }
            },
            &cancel,
        )
        .expect("import");
        println!(
            "import: {:.1}s — {} books ({} live), {} authors, {} series",
            t.elapsed().as_secs_f64(),
            stats.books,
            stats.live_books,
            stats.authors,
            stats.series
        );
        if lib_dir.is_some() {
            println!(
                "  offsets resolved: {}, missing archives: {}",
                stats.offsets_resolved,
                stats.missing_archives.len()
            );
        }
        for (phase, ms) in &stats.timings {
            println!("  {phase:<24} {:>8.1}s", *ms as f64 / 1000.0);
        }
        println!(
            "  db size: {} MB",
            std::fs::metadata(&db)
                .map(|m| m.len() / 1_000_000)
                .unwrap_or(0)
        );
    }

    let t = Instant::now();
    let cat = Catalog::open(&db).expect("open catalog");
    println!("open: {:.2} ms", ms(t.elapsed()));
    let st = cat.stats().clone();
    let mut rng = Rng::new(7);

    println!(
        "\n| {:<44} | {:>5} | {:>8} | {:>8} | {:>8} |",
        "query", "n", "p50 ms", "p95 ms", "max ms"
    );
    println!(
        "|{:-<46}|{:-<7}|{:-<10}|{:-<10}|{:-<10}|",
        "", "", "", "", ""
    );

    let mut s = Series::new("search: attrs load (once per catalog)");
    s.time(|| cat.attrs().unwrap());
    s.report();
    println!(
        "|   (attrs memory: {} MB) | | | | |",
        cat.attrs().unwrap().memory_bytes() / 1_000_000
    );

    let mut s = Series::new("authors list (first call)");
    let authors = s.time(|| cat.authors().unwrap());
    s.report();
    let mut s = Series::new("authors list + JSON (warm)");
    let mut json_len = 0;
    for _ in 0..5 {
        s.time(|| {
            let a = cat.authors().unwrap();
            json_len = serde_json::to_vec(&a).unwrap().len();
        });
    }
    s.report();
    let mut s = Series::new("series list + JSON");
    for _ in 0..5 {
        s.time(|| {
            serde_json::to_vec(&cat.series_list().unwrap())
                .unwrap()
                .len()
        });
    }
    s.report();
    let mut s = Series::new("genres with counts");
    for _ in 0..20 {
        s.time(|| cat.genres("ru").unwrap());
    }
    s.report();
    let mut s = Series::new("languages");
    for _ in 0..20 {
        s.time(|| cat.languages().unwrap());
    }
    s.report();

    let f = BookFilter::default();
    let page = Page::default();
    // Authors: the 100 biggest plus 300 random.
    let mut by_count: Vec<&(i64, String, i64)> = authors.rows.iter().collect();
    by_count.sort_by_key(|r| std::cmp::Reverse(r.2));
    let mut ids: Vec<i64> = by_count.iter().take(100).map(|r| r.0).collect();
    ids.extend((0..300).map(|_| authors.rows[rng.below(authors.rows.len())].0));
    let mut s = Series::new(&format!("books by author (top: {} books)", by_count[0].2));
    for id in &ids {
        s.time(|| cat.books(&BookSelector::Author(*id), &f, &page).unwrap());
    }
    s.report();
    let mut s = Series::new("books by author, lang=ru ext=fb2");
    let fr = BookFilter {
        langs: vec!["ru".into()],
        ext: Some("fb2".into()),
        include_deleted: false,
        q: None,
    };
    for id in &ids {
        s.time(|| cat.books(&BookSelector::Author(*id), &fr, &page).unwrap());
    }
    s.report();

    let mut s = Series::new("books by series");
    for _ in 0..300 {
        let id = 1 + rng.below(st.series_count as usize) as i64;
        s.time(|| cat.books(&BookSelector::Series(id), &f, &page).unwrap());
    }
    s.report();

    let genres = cat.genres("ru").unwrap();
    let mut s = Series::new("books by genre (leaf, first 2000)");
    let mut biggest = 0;
    for g in genres.iter().filter(|g| g.parent != 0 && g.count > 0) {
        biggest = biggest.max(g.count);
        s.time(|| cat.books(&BookSelector::Genre(g.id), &f, &page).unwrap());
    }
    s.report();
    println!("|   (largest leaf genre: {biggest} books) | | | | |");
    let mut s = Series::new("books by genre (top-level group)");
    for g in genres.iter().filter(|g| g.parent == 0 && g.count > 0) {
        s.time(|| cat.books(&BookSelector::Genre(g.id), &f, &page).unwrap());
    }
    s.report();
    let mut s = Series::new("books by genre, page 5 (cursor 8000)");
    let big = genres
        .iter()
        .filter(|g| g.parent != 0)
        .max_by_key(|g| g.count)
        .unwrap();
    for _ in 0..10 {
        s.time(|| {
            cat.books(
                &BookSelector::Genre(big.id),
                &f,
                &Page {
                    cursor: Some("8000".into()),
                    limit: 2000,
                },
            )
            .unwrap()
        });
    }
    s.report();

    for (label, date) in [
        ("since last 30 days", "2026-08-02"),
        ("since last year", "2025-09-01"),
    ] {
        let mut s = Series::new(&format!("books {label}"));
        for _ in 0..10 {
            s.time(|| {
                cat.books(&BookSelector::Since(date.into()), &f, &page)
                    .unwrap()
            });
        }
        s.report();
    }

    let mut s = Series::new("books by id list (shelf, 500 ids)");
    for _ in 0..20 {
        let ids: Vec<i64> = (0..500)
            .map(|_| 1 + rng.below(st.book_count as usize) as i64)
            .collect();
        s.time(|| cat.books(&BookSelector::Ids(ids), &f, &page).unwrap());
    }
    s.report();

    let mut s = Series::new("book by id (detail)");
    for _ in 0..2000 {
        let id = 1 + rng.below(st.book_count as usize) as i64;
        s.time(|| cat.book(id).unwrap());
    }
    s.report();

    let mut s = Series::new("ids by book_key (1000 keys)");
    for _ in 0..20 {
        let keys: Vec<String> = (0..1000)
            .map(|_| format!("lib:{}", 1000 + rng.below(st.book_count as usize)))
            .collect();
        s.time(|| cat.ids_by_keys(&keys).unwrap());
    }
    s.report();

    let queries = [
        "иванов",
        "тёмный лес",
        "стругацкий",
        "звёздный",
        "мир",
        "ал",
        "ив",
        "по",
        "хроники",
        "лукьяненко дозор",
        "последний рубеж",
        "king",
        "dark tower",
        "сергей",
        "петрова анна",
        "мёртвый",
        "огненный меч",
        "код",
        "шторм",
        "дело",
        "магия",
        "книга 3",
        "замок",
        "морозов",
        "сага о",
    ];
    let mut s = Series::new("search (kind=all, 25 queries × 3)");
    let mut sb = Series::new("search books only, lang=ru, genre=sf group");
    let mut max_total = 0;
    for q in queries {
        for _ in 0..3 {
            let r = s.time(|| {
                cat.search(&SearchQuery {
                    q: q.into(),
                    limit: 200,
                    ..Default::default()
                })
                .unwrap()
            });
            max_total = max_total.max(r.total);
            sb.time(|| {
                cat.search(&SearchQuery {
                    q: q.into(),
                    kind: SearchKind::Books,
                    langs: vec!["ru".into()],
                    genres: vec![1],
                    limit: 200,
                    ..Default::default()
                })
                .unwrap()
            });
        }
    }
    s.report();
    sb.report();
    println!("|   (largest match set: {max_total} books) | | | | |");

    let mut s = Series::new("search: vocabulary load (once per catalog)");
    let vocab = s.time(|| cat.vocab().unwrap());
    s.report();
    println!(
        "|   (vocabulary: {} words, {} MB) | | | | |",
        vocab.len(),
        vocab.memory_bytes() / 1_000_000
    );
    // word forms, transliterations and typos (the typo ones run the correction as well)
    let fuzzy = [
        "книгу",
        "мирами",
        "дозоры",
        "звёздных",
        "хроник",
        "лукьяненка",
        "strugatsky",
        "strugackie",
        "azimov",
        "shekli",
        "tolstoi",
        "dark towers",
        "стругацкй",
        "азимв",
        "лукяненко",
        "sheckley robrt",
    ];
    let mut s = Series::new("search: word forms / translit (10 queries × 3)");
    let mut t = Series::new("search: typos, corrected (6 queries × 3)");
    for (i, q) in fuzzy.iter().enumerate() {
        for _ in 0..3 {
            let sq = SearchQuery {
                q: (*q).into(),
                limit: 200,
                group: true,
                ..Default::default()
            };
            if i < 10 {
                s.time(|| cat.search(&sq).unwrap());
            } else {
                let r = t.time(|| cat.search(&sq).unwrap());
                if r.corrected.is_none() && r.did_you_mean.is_none() {
                    eprintln!("  no correction for {q}");
                }
            }
        }
    }
    s.report();
    t.report();
    let mut s = Series::new("search grouped by work (25 queries × 3)");
    let mut all = Series::new("search, all 41 queries (p95 target)");
    for q in queries.iter().chain(fuzzy.iter()) {
        for _ in 0..3 {
            let sq = SearchQuery {
                q: (*q).into(),
                limit: 200,
                group: true,
                ..Default::default()
            };
            let d = Instant::now();
            s.time(|| cat.search(&sq).unwrap());
            all.samples.push(ms(d.elapsed()));
        }
    }
    s.report();
    all.report();

    // grouped lists (editions of one work as one row)
    let nr = freelib_catalog::NoRatings;
    let rq = freelib_catalog::RatingQuery::default();
    let mut s = Series::new("grouped: books by author (400 authors)");
    for id in &ids {
        s.time(|| {
            cat.books_page(&BookSelector::Author(*id), &f, &rq, &nr, &page, true)
                .unwrap()
        });
    }
    s.report();
    for (label, date) in [
        ("since 30 days", "2026-08-02"),
        ("since 1900 (all)", "1900-01-01"),
    ] {
        let mut s = Series::new(&format!("grouped: books {label}"));
        for _ in 0..5 {
            s.time(|| {
                cat.books_page(&BookSelector::Since(date.into()), &f, &rq, &nr, &page, true)
                    .unwrap()
            });
        }
        s.report();
    }
    let top = genres
        .iter()
        .filter(|g| g.parent == 0)
        .max_by_key(|g| g.count)
        .unwrap();
    let mut s = Series::new("grouped: biggest top-level genre");
    for _ in 0..5 {
        s.time(|| {
            cat.books_page(&BookSelector::Genre(top.id), &f, &rq, &nr, &page, true)
                .unwrap()
        });
    }
    s.report();

    // start page: 300 books read by a user, 50 read authors, 10 followed series
    let mut s = Series::new("start page: continue series (300 read)");
    let mut n = Series::new("start page: new from authors (30 days)");
    for round in 0..5 {
        let mut r = Rng::new(100 + round);
        let done: std::collections::HashMap<i64, String> = (0..300)
            .map(|i| {
                (
                    1 + r.below(st.book_count as usize) as i64,
                    format!("2026-09-{:02}T10:00:00Z", 1 + i % 28),
                )
            })
            .collect();
        s.time(|| {
            cat.continue_series(&done, &Default::default(), &nr, 2, 24)
                .unwrap()
        });
        let read: Vec<i64> = done.keys().copied().collect();
        let seeds = freelib_catalog::home::NewFromSeeds {
            read_authors: cat.reading_authors(&read).unwrap(),
            followed_series: (0..10)
                .map(|_| 1 + r.below(st.series_count as usize) as i64)
                .collect(),
            ..Default::default()
        };
        let exclude = done.keys().copied().collect();
        n.time(|| {
            cat.new_from(&seeds, "2026-08-02", &exclude, &nr, 60)
                .unwrap()
        });
    }
    s.report();
    n.report();
    println!(
        "\nauthors JSON: {} KB for {} rows",
        json_len / 1000,
        authors.rows.len()
    );
}
