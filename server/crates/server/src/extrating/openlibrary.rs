//! Open Library lookups: `search.json` by title + author surname, a conservative match, then
//! the work's `ratings.json` (average + count).
//!
//! Matching prefers no match over a wrong one: the normalised title (or its part before a
//! `:`, with a leading English article dropped) must be equal, and one of the book's author
//! surnames must be a word of one of the result's author names. Cyrillic titles and names are
//! also tried transliterated; comparisons use a Latin "key" that folds the usual
//! transliteration variants (`Strugatsky` = `Strugatskii` = `Стругацкий`).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use tokio::time::Instant;

/// Result of an HTTP GET: status and body, or a transport error.
pub type HttpResult = Result<(u16, String), String>;

/// The HTTP layer (a trait so tests can answer from fixtures without a network).
pub trait HttpGet: Send + Sync + 'static {
    fn get(&self, url: String) -> Pin<Box<dyn Future<Output = HttpResult> + Send>>;
}

/// `reqwest` with a descriptive User-Agent and a 20 s timeout.
pub struct ReqwestGet {
    client: reqwest::Client,
}

impl ReqwestGet {
    pub fn new(user_agent: &str) -> Result<ReqwestGet, String> {
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(ReqwestGet { client })
    }
}

impl HttpGet for ReqwestGet {
    fn get(&self, url: String) -> Pin<Box<dyn Future<Output = HttpResult> + Send>> {
        let c = self.client.clone();
        Box::pin(async move {
            let r = c
                .get(&url)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = r.status().as_u16();
            // bodies are small JSON; bound them anyway
            let body = r.bytes().await.map_err(|e| e.to_string())?;
            if body.len() > 4 * 1024 * 1024 {
                return Err("response too large".into());
            }
            Ok((status, String::from_utf8_lossy(&body).into_owned()))
        })
    }
}

/// The User-Agent Open Library asks for: application name, version and a contact.
pub fn user_agent(contact: Option<&str>) -> String {
    let contact = contact.map(|c| format!("; {c}")).unwrap_or_default();
    format!(
        "freeLib/{} (+https://github.com/Kwull/freeLib{contact})",
        env!("CARGO_PKG_VERSION")
    )
}

/// One request at a time, at least `interval` apart; after errors a growing pause
/// (30 s, 60 s, … 1 h) before the next request.
pub struct Limiter {
    interval: Duration,
    state: tokio::sync::Mutex<Instant>,
    backoff: Mutex<(u32, Option<Instant>)>,
}

/// Longest pause after repeated errors.
pub const MAX_BACKOFF: Duration = Duration::from_secs(3600);
const FIRST_BACKOFF: Duration = Duration::from_secs(30);

impl Limiter {
    pub fn new(interval: Duration) -> Limiter {
        Limiter {
            interval,
            state: tokio::sync::Mutex::new(Instant::now()),
            backoff: Mutex::new((0, None)),
        }
    }

    /// Waits for the next request slot.
    pub async fn acquire(&self) {
        let mut next = self.state.lock().await;
        let now = Instant::now();
        let mut at = (*next).max(now);
        if let Some(b) = self.backoff_until() {
            at = at.max(b);
        }
        if at > now {
            tokio::time::sleep_until(at).await;
        }
        *next = at + self.interval;
    }

    /// When requests may resume after errors (`None` = no pause).
    pub fn backoff_until(&self) -> Option<Instant> {
        let g = self.backoff.lock().unwrap_or_else(|e| e.into_inner());
        g.1.filter(|t| *t > Instant::now())
    }

    /// A failed request (5xx, 429, network): pause, doubling with each consecutive failure.
    pub fn failed(&self) -> Duration {
        let mut g = self.backoff.lock().unwrap_or_else(|e| e.into_inner());
        g.0 = g.0.saturating_add(1);
        let pause = FIRST_BACKOFF
            .saturating_mul(1u32 << (g.0 - 1).min(10))
            .min(MAX_BACKOFF);
        g.1 = Some(Instant::now() + pause);
        pause
    }

    pub fn succeeded(&self) {
        let mut g = self.backoff.lock().unwrap_or_else(|e| e.into_inner());
        *g = (0, None);
    }
}

/// What to look up.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub title: String,
    /// Surnames (INPX last names) of the book's authors, first author first.
    pub surnames: Vec<String>,
}

/// Result of a lookup.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Found {
        /// `/works/OL…W`
        work_key: String,
        /// `None` when the work has no ratings.
        average: Option<f64>,
        count: u32,
    },
    NotFound,
    /// Transport / server error; retried later.
    Error(String),
}

#[derive(Debug, Deserialize)]
struct SearchResp {
    #[serde(default)]
    docs: Vec<Doc>,
}

#[derive(Debug, Deserialize)]
struct Doc {
    key: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    author_name: Vec<String>,
    #[serde(default)]
    ratings_count: Option<u32>,
    #[serde(default)]
    edition_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RatingsResp {
    summary: RatingsSummary,
}

#[derive(Debug, Deserialize)]
struct RatingsSummary {
    #[serde(default)]
    average: Option<f64>,
    #[serde(default)]
    count: Option<u32>,
}

/// The Open Library client.
pub struct OpenLibrary {
    base: String,
    http: Arc<dyn HttpGet>,
    pub limiter: Arc<Limiter>,
}

enum Fetch {
    Ok(String),
    NotFound,
    Err(String),
}

impl OpenLibrary {
    pub fn new(base: &str, http: Arc<dyn HttpGet>, limiter: Arc<Limiter>) -> OpenLibrary {
        OpenLibrary {
            base: base.trim_end_matches('/').to_string(),
            http,
            limiter,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    async fn fetch(&self, url: String) -> Fetch {
        self.limiter.acquire().await;
        match self.http.get(url).await {
            Ok((200, body)) => {
                self.limiter.succeeded();
                Fetch::Ok(body)
            }
            Ok((404, _)) => {
                self.limiter.succeeded();
                Fetch::NotFound
            }
            Ok((s, _)) if s == 429 || s >= 500 => {
                let p = self.limiter.failed();
                Fetch::Err(format!("HTTP {s}; pausing {} s", p.as_secs()))
            }
            Ok((s, _)) => Fetch::Err(format!("HTTP {s}")),
            Err(e) => {
                let p = self.limiter.failed();
                Fetch::Err(format!("{e}; pausing {} s", p.as_secs()))
            }
        }
    }

    /// Searches, matches and fetches the ratings of the best matching work.
    pub async fn lookup(&self, q: &Query) -> Outcome {
        let title = clean_title(&q.title);
        let surnames: Vec<&String> = q.surnames.iter().filter(|s| !s.trim().is_empty()).collect();
        if title_key(&title).chars().count() < 2 || surnames.is_empty() {
            return Outcome::NotFound; // nothing to match on safely
        }
        let first = surnames[0].trim();
        let mut attempts: Vec<(String, String)> = vec![(title.clone(), first.to_string())];
        if has_cyrillic(first) {
            attempts.push((title.clone(), translit(first)));
        }
        if has_cyrillic(&title) {
            attempts.push((translit(&title), translit(first)));
        }
        let mut last_err = None;
        for (t, a) in attempts {
            let url = format!(
                "{}/search.json?title={}&author={}&fields=key,title,author_name,ratings_count,edition_count&limit=20",
                self.base,
                enc(&t),
                enc(&a)
            );
            let body = match self.fetch(url).await {
                Fetch::Ok(b) => b,
                Fetch::NotFound => continue,
                Fetch::Err(e) => {
                    last_err = Some(e);
                    break;
                }
            };
            let Ok(resp) = serde_json::from_str::<SearchResp>(&body) else {
                last_err = Some("unexpected search response".into());
                break;
            };
            if let Some(doc) = best_match(&resp.docs, &q.title, &q.surnames) {
                return self.ratings(&doc.key).await;
            }
        }
        match last_err {
            Some(e) => Outcome::Error(e),
            None => Outcome::NotFound,
        }
    }

    /// `ratings.json` of a work (`/works/OL…W`).
    pub async fn ratings(&self, work_key: &str) -> Outcome {
        if !valid_work_key(work_key) {
            return Outcome::Error(format!("unexpected work key {work_key}"));
        }
        let url = format!("{}{work_key}/ratings.json", self.base);
        match self.fetch(url).await {
            Fetch::Ok(body) => match serde_json::from_str::<RatingsResp>(&body) {
                Ok(r) => {
                    let count = r.summary.count.unwrap_or(0);
                    Outcome::Found {
                        work_key: work_key.to_string(),
                        average: r.summary.average.filter(|a| count > 0 && a.is_finite()),
                        count,
                    }
                }
                Err(_) => Outcome::Error("unexpected ratings response".into()),
            },
            Fetch::NotFound => Outcome::Found {
                work_key: work_key.to_string(),
                average: None,
                count: 0,
            },
            Fetch::Err(e) => Outcome::Error(e),
        }
    }
}

/// `/works/OL123W`
pub fn valid_work_key(k: &str) -> bool {
    k.strip_prefix("/works/OL")
        .and_then(|r| r.strip_suffix('W'))
        .is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()))
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// The best document whose title and an author surname match (most ratings, then editions).
fn best_match<'a>(docs: &'a [Doc], title: &str, surnames: &[String]) -> Option<&'a Doc> {
    docs.iter()
        .filter(|d| d.key.starts_with("/works/") && titles_match(title, &d.title))
        .filter(|d| authors_match(surnames, &d.author_name))
        .max_by_key(|d| (d.ratings_count.unwrap_or(0), d.edition_count.unwrap_or(0)))
}

/// Drops bracketed notes (`(сборник)`, `[litres]`) and surrounding punctuation.
pub fn clean_title(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut depth = 0i32;
    for c in t.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let s = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    if s.is_empty() {
        t.trim().to_string()
    } else {
        s
    }
}

pub fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

pub use freelib_catalog::text::{translit, word_key};

/// Title comparison key: words keyed, a leading English article dropped.
pub fn title_key(t: &str) -> String {
    let n = freelib_catalog::normalize(&translit(t));
    let mut words: Vec<String> = n
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(word_key)
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() > 1 && matches!(words[0].as_str(), "the" | "a" | "an") {
        words.remove(0);
    }
    words.join(" ")
}

/// The part of a title before a subtitle separator (`:`, ` - `, `. `).
fn main_title(t: &str) -> &str {
    let cut = [":", " - ", " — ", ". "]
        .iter()
        .filter_map(|sep| t.find(sep))
        .min();
    match cut {
        Some(i) if i > 0 => &t[..i],
        _ => t,
    }
}

/// Whether a book title and an Open Library title name the same work (conservative).
pub fn titles_match(book: &str, doc: &str) -> bool {
    let b = title_key(&clean_title(book));
    let d = title_key(doc);
    if b.is_empty() || d.is_empty() {
        return false;
    }
    if b == d {
        return true;
    }
    // "Solaris: a novel" ~ "Solaris"; the shared main title must be meaningful
    let bm = title_key(main_title(&clean_title(book)));
    let dm = title_key(main_title(doc));
    let long = |k: &str| k.chars().filter(|c| *c != ' ').count() >= 4;
    (bm == d && long(&bm)) || (dm == b && long(&dm))
}

/// Whether one of `surnames` is a word of one of the Open Library author names.
pub fn authors_match(surnames: &[String], names: &[String]) -> bool {
    let keys: Vec<String> = surnames
        .iter()
        .flat_map(|s| {
            freelib_catalog::normalize(s)
                .split(' ')
                .map(word_key)
                .filter(|k| k.chars().count() >= 2)
                .collect::<Vec<_>>()
        })
        .collect();
    if keys.is_empty() {
        return false;
    }
    names.iter().any(|n| {
        freelib_catalog::normalize(n)
            .split(' ')
            .map(word_key)
            .any(|w| keys.contains(&w))
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn keys_fold_variants() {
        for (a, b) in [
            ("Strugatsky", "Стругацкий"),
            ("Strugatskii", "Strugatskiy"),
            ("Dostoevsky", "Dostoyevsky"),
            ("Dostoevskii", "Достоевский"),
            ("Tolstoy", "Tolstoi"),
            ("Толстой", "Tolstoy"),
            ("Lukyanenko", "Lukianenko"),
            ("Лукьяненко", "Lukyanenko"),
            ("Chekhov", "Tchekhov"),
            ("Чехов", "Chekhov"),
            ("Yefremov", "Ефремов"),
            ("Aksyonov", "Aksenov"),
        ] {
            let (ka, kb) = (word_key(a), word_key(b));
            if (a, b) == ("Aksyonov", "Aksenov") {
                // ё is written "yo" in some schemes; not folded (a miss, not a wrong match)
                assert_ne!(ka, kb);
                continue;
            }
            assert_eq!(ka, kb, "{a} / {b}");
        }
        assert_ne!(word_key("Tolstoy"), word_key("Tolstaya"));
        assert_ne!(word_key("Strugatsky"), word_key("Stratsky"));
    }

    #[test]
    fn titles() {
        assert!(titles_match("Пикник на обочине", "Piknik na obochine"));
        assert!(titles_match("The Lord of the Rings", "Lord of the Rings"));
        assert!(titles_match("Solaris (сборник)", "Solaris"));
        assert!(titles_match("Dune: Messiah", "Dune Messiah"));
        assert!(titles_match(
            "Foundation and Empire",
            "Foundation and empire"
        ));
        assert!(!titles_match("Dune", "Dune Messiah"));
        assert!(!titles_match("It", "It Ends With Us"));
        assert!(!titles_match("Мастер и Маргарита", "Мастер"));
        assert!(!titles_match(
            "Collected stories: vol 1",
            "Collected stories: vol 2"
        ));
    }

    #[test]
    fn authors() {
        let s = vec!["Стругацкий".to_string()];
        assert!(authors_match(
            &s,
            &["Arkady Strugatsky".into(), "Boris Strugatsky".into()]
        ));
        assert!(authors_match(&s, &["Arkadiĭ Strugat͡skiĭ".into()]));
        assert!(!authors_match(&s, &["Isaac Asimov".into()]));
        assert!(!authors_match(&[], &["Isaac Asimov".into()]));
        assert!(authors_match(
            &["Le Guin".into()],
            &["Ursula K. Le Guin".into()]
        ));
    }

    /// Answers from recorded fixtures: the first route whose needle is in the URL.
    pub(crate) struct FakeHttp {
        pub routes: Vec<(&'static str, u16, &'static str)>,
        pub calls: Mutex<Vec<(String, Instant)>>,
    }

    impl FakeHttp {
        pub fn new(routes: Vec<(&'static str, u16, &'static str)>) -> Arc<FakeHttp> {
            Arc::new(FakeHttp {
                routes,
                calls: Mutex::new(Vec::new()),
            })
        }
        pub fn urls(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| c.0.clone())
                .collect()
        }
    }

    impl HttpGet for FakeHttp {
        fn get(&self, url: String) -> Pin<Box<dyn Future<Output = HttpResult> + Send>> {
            self.calls
                .lock()
                .unwrap()
                .push((url.clone(), Instant::now()));
            let r = self
                .routes
                .iter()
                .find(|(needle, _, _)| url.contains(needle))
                .map(|(_, s, b)| Ok((*s, b.to_string())))
                .unwrap_or(Ok((404, String::new())));
            Box::pin(async move { r })
        }
    }

    const PIKNIK: &str = include_str!("../../tests/fixtures/openlibrary/search_piknik.json");
    const PIKNIK_R: &str = include_str!("../../tests/fixtures/openlibrary/ratings_OL1914203W.json");
    const DUNE: &str = include_str!("../../tests/fixtures/openlibrary/search_dune.json");
    const DUNE_R: &str = include_str!("../../tests/fixtures/openlibrary/ratings_OL893415W.json");
    const IT: &str = include_str!("../../tests/fixtures/openlibrary/search_it.json");
    const EMPTY: &str = include_str!("../../tests/fixtures/openlibrary/search_empty.json");
    const NONE_R: &str = include_str!("../../tests/fixtures/openlibrary/ratings_none.json");

    fn client(http: Arc<FakeHttp>) -> OpenLibrary {
        OpenLibrary::new(
            "https://ol.test/",
            http,
            Arc::new(Limiter::new(Duration::from_secs(1))),
        )
    }

    fn q(title: &str, surname: &str) -> Query {
        Query {
            title: title.into(),
            surnames: vec![surname.into()],
        }
    }

    #[tokio::test(start_paused = true)]
    async fn cyrillic_book_found() {
        let http = FakeHttp::new(vec![
            ("/search.json", 200, PIKNIK),
            ("/works/OL1914203W/ratings.json", 200, PIKNIK_R),
        ]);
        let ol = client(http.clone());
        let out = ol.lookup(&q("Пикник на обочине", "Стругацкий")).await;
        // the exact title with the right authors, not the omnibus nor the namesake with more ratings
        assert_eq!(
            out,
            Outcome::Found {
                work_key: "/works/OL1914203W".into(),
                average: Some(4.25),
                count: 16
            }
        );
        let urls = http.urls();
        assert_eq!(urls.len(), 2);
        assert!(
            urls[0].starts_with("https://ol.test/search.json?title="),
            "{}",
            urls[0]
        );
        assert!(urls[0].contains("fields=key%2Ctitle") || urls[0].contains("fields=key,title"));
        assert_eq!(urls[1], "https://ol.test/works/OL1914203W/ratings.json");
    }

    #[tokio::test(start_paused = true)]
    async fn prefers_the_matching_author() {
        let http = FakeHttp::new(vec![
            ("/search.json", 200, DUNE),
            ("/works/OL893415W/ratings.json", 200, DUNE_R),
        ]);
        let out = client(http).lookup(&q("Dune", "Herbert")).await;
        assert_eq!(
            out,
            Outcome::Found {
                work_key: "/works/OL893415W".into(),
                average: Some(4.31),
                count: 1203
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn no_match_is_not_found() {
        // "It" by King: only "It Ends with Us" and an "It" by another author come back
        let http = FakeHttp::new(vec![("/search.json", 200, IT)]);
        let out = client(http.clone()).lookup(&q("It", "King")).await;
        assert_eq!(out, Outcome::NotFound);
        assert_eq!(http.urls().len(), 1, "no ratings request without a match");
        // Cyrillic title and author: original, transliterated author, transliterated both
        let http = FakeHttp::new(vec![("/search.json", 200, EMPTY)]);
        let out = client(http.clone())
            .lookup(&q("Понедельник", "Стругацкий"))
            .await;
        assert_eq!(out, Outcome::NotFound);
        let urls = http.urls();
        assert_eq!(urls.len(), 3, "{urls:?}");
        assert!(urls[1].contains("author=Strugatskiy"), "{}", urls[1]);
        assert!(urls[2].contains("title=Ponedelnik"), "{}", urls[2]);
        // nothing safe to match on: no request at all
        let http = FakeHttp::new(vec![]);
        assert_eq!(
            client(http.clone()).lookup(&q("Dune", "")).await,
            Outcome::NotFound
        );
        assert_eq!(
            client(http.clone()).lookup(&q("X", "Herbert")).await,
            Outcome::NotFound
        );
        assert!(http.urls().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn work_without_ratings_and_errors() {
        let http = FakeHttp::new(vec![
            ("/search.json", 200, DUNE),
            ("/ratings.json", 200, NONE_R),
        ]);
        let out = client(http).lookup(&q("Dune", "Herbert")).await;
        assert_eq!(
            out,
            Outcome::Found {
                work_key: "/works/OL893415W".into(),
                average: None,
                count: 0
            }
        );
        let http = FakeHttp::new(vec![("/search.json", 503, "busy")]);
        let ol = client(http.clone());
        let out = ol.lookup(&q("Dune", "Herbert")).await;
        assert!(
            matches!(out, Outcome::Error(ref e) if e.contains("503")),
            "{out:?}"
        );
        // the error paused further requests
        let paused = ol.limiter.backoff_until().expect("backoff");
        assert!(paused > Instant::now() + Duration::from_secs(25));
    }

    #[tokio::test(start_paused = true)]
    async fn limiter_spaces_requests_and_backs_off() {
        let l = Limiter::new(Duration::from_secs(1));
        let t0 = Instant::now();
        let mut at = Vec::new();
        for _ in 0..4 {
            l.acquire().await;
            at.push(Instant::now() - t0);
        }
        assert_eq!(at[0], Duration::ZERO);
        for w in at.windows(2) {
            assert!(w[1] - w[0] >= Duration::from_secs(1), "{at:?}");
        }
        assert_eq!(l.failed(), Duration::from_secs(30));
        assert_eq!(l.failed(), Duration::from_secs(60));
        assert_eq!(l.failed(), Duration::from_secs(120));
        for _ in 0..20 {
            l.failed();
        }
        assert_eq!(l.failed(), MAX_BACKOFF);
        let before = Instant::now();
        l.acquire().await;
        assert!(Instant::now() - before >= MAX_BACKOFF - Duration::from_secs(1));
        l.succeeded();
        assert!(l.backoff_until().is_none());
    }

    #[test]
    fn misc() {
        assert!(valid_work_key("/works/OL45883W"));
        assert!(!valid_work_key("/works/OL45883M"));
        assert!(!valid_work_key("/../evil"));
        assert_eq!(translit("Щука Ёж"), "Shchuka Ezh");
        assert_eq!(clean_title("  Солярис [litres] "), "Солярис");
        assert!(user_agent(Some("me@example.org")).contains("me@example.org"));
        assert!(user_agent(None).starts_with("freeLib/"));
    }
}
