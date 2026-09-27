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

/// An HTTP answer (any status).
#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    /// `Retry-After` in seconds (429 / 503), when the server sent one.
    pub retry_after: Option<u64>,
}

impl HttpResponse {
    pub fn new(status: u16, body: impl Into<String>) -> HttpResponse {
        HttpResponse {
            status,
            body: body.into(),
            retry_after: None,
        }
    }
}

/// What kind of transport failure a request had (the pause after it depends on it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The host name did not resolve (no DNS in the container, a typo in the URL).
    Dns,
    /// No connection (refused, unreachable, blocked by a firewall).
    Connect,
    /// The TLS handshake failed (a proxy or middlebox with its own certificate).
    Tls,
    /// The HTTP(S) proxy refused or failed.
    Proxy,
    /// No answer in time.
    Timeout,
    /// Anything else (a broken body, …).
    Other,
}

impl ErrorKind {
    pub fn label(self) -> &'static str {
        match self {
            ErrorKind::Dns => "DNS",
            ErrorKind::Connect => "connection",
            ErrorKind::Tls => "TLS",
            ErrorKind::Proxy => "proxy",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Other => "network",
        }
    }
}

/// A transport error: its kind and the full error chain.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpError {
    pub kind: ErrorKind,
    pub detail: String,
}

/// Result of an HTTP GET.
pub type HttpResult = Result<HttpResponse, HttpError>;

/// The HTTP layer (a trait so tests can answer from fixtures without a network).
pub trait HttpGet: Send + Sync + 'static {
    fn get(&self, url: String) -> Pin<Box<dyn Future<Output = HttpResult> + Send>>;
}

/// An error with its sources (`reqwest` hides the interesting part in them).
pub fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        let t = c.to_string();
        if !s.contains(&t) {
            s.push_str(": ");
            s.push_str(&t);
        }
        cur = c.source();
    }
    s
}

/// Sorts a `reqwest` error into an [`ErrorKind`] by its flags and its error chain.
pub fn classify(e: &reqwest::Error) -> HttpError {
    let detail = error_chain(e);
    let lower = detail.to_lowercase();
    let kind = if e.is_timeout() || lower.contains("timed out") {
        ErrorKind::Timeout
    } else if lower.contains("dns error")
        || lower.contains("failed to lookup address")
        || lower.contains("name or service not known")
        || lower.contains("no such host")
        || lower.contains("temporary failure in name resolution")
    {
        ErrorKind::Dns
    } else if lower.contains("proxy") {
        ErrorKind::Proxy
    } else if lower.contains("certificate")
        || lower.contains("tls")
        || lower.contains("handshake")
        || lower.contains("invalidcertificate")
    {
        ErrorKind::Tls
    } else if e.is_connect() {
        ErrorKind::Connect
    } else {
        ErrorKind::Other
    };
    HttpError { kind, detail }
}

/// The HTTPS proxy `reqwest` takes from the environment (`HTTPS_PROXY`, `ALL_PROXY`,
/// `HTTP_PROXY`, lower-case too), without credentials, for the log; `None` without one.
pub fn env_proxy() -> Option<String> {
    [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .iter()
    .filter_map(|k| std::env::var(k).ok())
    .find(|v| !v.trim().is_empty())
    .map(|v| match (v.find("://"), v.rfind('@')) {
        (Some(i), Some(at)) if at > i => format!("{}{}", &v[..i + 3], &v[at + 1..]),
        _ => v,
    })
}

/// `reqwest` with a descriptive User-Agent, a 10 s connect and a 20 s total timeout, kept-alive
/// connections (one host, one request at a time), and the proxy of `HTTPS_PROXY` /
/// `HTTP_PROXY` / `ALL_PROXY` (with `NO_PROXY`), as `reqwest` reads them.
pub struct ReqwestGet {
    client: reqwest::Client,
}

impl ReqwestGet {
    pub fn new(user_agent: &str) -> Result<ReqwestGet, String> {
        Self::build(
            user_agent,
            Duration::from_secs(10),
            Duration::from_secs(20),
            true,
        )
    }

    /// [`new`](Self::new) with other timeouts, and optionally without the environment's proxy.
    pub fn build(
        user_agent: &str,
        connect_timeout: Duration,
        timeout: Duration,
        env_proxy: bool,
    ) -> Result<ReqwestGet, String> {
        let mut b = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(connect_timeout)
            .timeout(timeout)
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(2)
            .tcp_keepalive(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::limited(3));
        if !env_proxy {
            b = b.no_proxy();
        }
        let client = b.build().map_err(|e| error_chain(&e))?;
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
                .map_err(|e| classify(&e))?;
            let status = r.status().as_u16();
            let retry_after = r
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            // bodies are small JSON; bound them anyway
            let body = r.bytes().await.map_err(|e| classify(&e))?;
            if body.len() > 4 * 1024 * 1024 {
                return Err(HttpError {
                    kind: ErrorKind::Other,
                    detail: "response too large".into(),
                });
            }
            Ok(HttpResponse {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
                retry_after,
            })
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

/// Why requests pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// HTTP 429 (with the server's `Retry-After`, if any).
    TooMany(Option<u64>),
    /// HTTP 5xx.
    Server,
    /// A transport error of this kind.
    Transport(ErrorKind),
}

impl Failure {
    /// (first pause, longest pause) of this kind; the pause doubles with every consecutive
    /// failure up to the longest.
    fn backoff(self) -> (Duration, Duration) {
        match self {
            Failure::TooMany(_) => (Duration::from_secs(60), MAX_BACKOFF),
            Failure::Server => (FIRST_BACKOFF, Duration::from_secs(30 * 60)),
            Failure::Transport(ErrorKind::Timeout) => {
                (Duration::from_secs(15), Duration::from_secs(15 * 60))
            }
            Failure::Transport(ErrorKind::Other) => (FIRST_BACKOFF, Duration::from_secs(15 * 60)),
            Failure::Transport(_) => (FIRST_BACKOFF, MAX_BACKOFF),
        }
    }
}

/// One request at a time, at least `interval` apart; after errors a pause that doubles with
/// each consecutive failure, from a first pause and up to a cap that depend on the kind of
/// failure ([`Failure`]: 429 honours `Retry-After`).
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

    /// A failed request (5xx, network): pause as a generic server error ([`Failure::Server`]).
    pub fn failed(&self) -> Duration {
        self.failed_with(Failure::Server)
    }

    /// A failed request: pause, doubling with each consecutive failure, capped per kind.
    pub fn failed_with(&self, why: Failure) -> Duration {
        let mut g = self.backoff.lock().unwrap_or_else(|e| e.into_inner());
        g.0 = g.0.saturating_add(1);
        let (first, cap) = why.backoff();
        let mut pause = first.saturating_mul(1u32 << (g.0 - 1).min(10)).min(cap);
        if let Failure::TooMany(Some(secs)) = why {
            pause = pause.max(Duration::from_secs(secs).min(MAX_BACKOFF));
        }
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
    /// Valid ISBN-13s from the book's `<publish-info>` (tried before the title search).
    pub isbns: Vec<String>,
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

/// `/isbn/<isbn>.json`: an edition.
#[derive(Debug, Deserialize)]
struct EditionResp {
    #[serde(default)]
    title: String,
    #[serde(default)]
    works: Vec<KeyRef>,
}

#[derive(Debug, Deserialize)]
struct KeyRef {
    key: String,
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
            Ok(r) if r.status == 200 => {
                self.limiter.succeeded();
                Fetch::Ok(r.body)
            }
            Ok(r) if r.status == 404 => {
                self.limiter.succeeded();
                Fetch::NotFound
            }
            Ok(r) if r.status == 429 => {
                let p = self.limiter.failed_with(Failure::TooMany(r.retry_after));
                Fetch::Err(format!(
                    "HTTP 429 (too many requests){}; pausing {} s",
                    r.retry_after
                        .map(|s| format!(", Retry-After {s} s"))
                        .unwrap_or_default(),
                    p.as_secs()
                ))
            }
            Ok(r) if r.status >= 500 => {
                let p = self.limiter.failed_with(Failure::Server);
                Fetch::Err(format!(
                    "HTTP {} (server error); pausing {} s",
                    r.status,
                    p.as_secs()
                ))
            }
            Ok(r) => Fetch::Err(format!("HTTP {}", r.status)),
            Err(e) => {
                let p = self.limiter.failed_with(Failure::Transport(e.kind));
                Fetch::Err(format!(
                    "{} error: {}; pausing {} s",
                    e.kind.label(),
                    e.detail,
                    p.as_secs()
                ))
            }
        }
    }

    /// The work of an ISBN whose edition title matches the book (a wrong ISBN in the file
    /// must not attach another book's rating). `Ok(None)` = no usable answer.
    async fn by_isbn(&self, isbn: &str, title: &str) -> Result<Option<String>, String> {
        if !(isbn.len() == 13 && isbn.bytes().all(|b| b.is_ascii_digit())) {
            return Ok(None);
        }
        match self.fetch(format!("{}/isbn/{isbn}.json", self.base)).await {
            Fetch::Ok(body) => {
                let Ok(ed) = serde_json::from_str::<EditionResp>(&body) else {
                    return Ok(None);
                };
                Ok(ed
                    .works
                    .into_iter()
                    .map(|w| w.key)
                    .find(|k| valid_work_key(k))
                    .filter(|_| titles_match(title, &ed.title)))
            }
            Fetch::NotFound => Ok(None),
            Fetch::Err(e) => Err(e),
        }
    }

    /// Looks the book up by ISBN first, then searches by title and author, matches and
    /// fetches the ratings of the best matching work.
    pub async fn lookup(&self, q: &Query) -> Outcome {
        for isbn in q.isbns.iter().take(3) {
            match self.by_isbn(isbn, &q.title).await {
                Ok(Some(work)) => return self.ratings(&work).await,
                Ok(None) => {}
                Err(e) => return Outcome::Error(e),
            }
        }
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
        // "Солярис: роман" / "Dune. Book 1": the main title alone (matching still needs the
        // whole title or a meaningful main title, see `titles_match`)
        let main = main_title(&title).trim().to_string();
        if main != title && title_key(&main).chars().filter(|c| *c != ' ').count() >= 4 {
            attempts.push((main.clone(), first.to_string()));
            if has_cyrillic(&main) {
                attempts.push((translit(&main), translit(first)));
            }
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

pub use freelib_catalog::text::{phonetic_key, translit, word_key};

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

/// Whether one of `surnames` is a word of one of the Open Library author names, compared by
/// transliteration key or by the coarse phonetic key (`Азимов` = `Asimov`, `Tolstoj` =
/// `Tolstoy`).
pub fn authors_match(surnames: &[String], names: &[String]) -> bool {
    let keys: Vec<(String, String)> = surnames
        .iter()
        .flat_map(|s| {
            freelib_catalog::normalize(s)
                .split(' ')
                .map(|w| (word_key(w), phonetic_key(w)))
                .filter(|(k, _)| k.chars().count() >= 2)
                .collect::<Vec<_>>()
        })
        .collect();
    if keys.is_empty() {
        return false;
    }
    names.iter().any(|n| {
        freelib_catalog::normalize(n).split(' ').any(|w| {
            let (k, p) = (word_key(w), phonetic_key(w));
            keys.iter()
                .any(|(sk, sp)| *sk == k || (sp.chars().count() >= 4 && *sp == p))
        })
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
                .map(|(_, s, b)| Ok(HttpResponse::new(*s, *b)))
                .unwrap_or(Ok(HttpResponse::new(404, "")));
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
            isbns: Vec::new(),
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
    async fn isbn_first() {
        let http = FakeHttp::new(vec![
            (
                "/isbn/9785699120147.json",
                200,
                r#"{"title": "Пикник на обочине", "works": [{"key": "/works/OL1914203W"}]}"#,
            ),
            ("/works/OL1914203W/ratings.json", 200, PIKNIK_R),
        ]);
        let mut query = q("Пикник на обочине", "Стругацкий");
        query.isbns = vec!["9785699120147".into()];
        let out = client(http.clone()).lookup(&query).await;
        assert_eq!(
            out,
            Outcome::Found {
                work_key: "/works/OL1914203W".into(),
                average: Some(4.25),
                count: 16
            }
        );
        assert_eq!(
            http.urls(),
            [
                "https://ol.test/isbn/9785699120147.json",
                "https://ol.test/works/OL1914203W/ratings.json"
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn isbn_of_another_book_is_ignored() {
        // the file's ISBN belongs to another book: fall back to the title search
        let http = FakeHttp::new(vec![
            (
                "/isbn/9785699120147.json",
                200,
                r#"{"title": "Совсем другая книга", "works": [{"key": "/works/OL1W"}]}"#,
            ),
            ("/search.json", 200, PIKNIK),
            ("/works/OL1914203W/ratings.json", 200, PIKNIK_R),
        ]);
        let mut query = q("Пикник на обочине", "Стругацкий");
        query.isbns = vec!["9785699120147".into(), "not-an-isbn".into()];
        let out = client(http.clone()).lookup(&query).await;
        assert!(
            matches!(out, Outcome::Found { ref work_key, .. } if work_key == "/works/OL1914203W")
        );
        let urls = http.urls();
        assert_eq!(urls.len(), 3, "{urls:?}");
        assert!(urls[1].contains("/search.json"));
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
        // server errors: capped at 30 minutes
        assert_eq!(l.failed(), Duration::from_secs(1800));
        let before = Instant::now();
        l.acquire().await;
        assert!(Instant::now() - before >= Duration::from_secs(1799));
        l.succeeded();
        assert!(l.backoff_until().is_none());
    }

    #[test]
    fn backoff_depends_on_the_kind_of_failure() {
        let seq = |why: Failure, n: usize| {
            let l = Limiter::new(Duration::from_secs(1));
            (0..n)
                .map(|_| l.failed_with(why).as_secs())
                .collect::<Vec<_>>()
        };
        assert_eq!(seq(Failure::Server, 3), [30, 60, 120]);
        assert_eq!(*seq(Failure::Server, 20).last().unwrap(), 1800);
        assert_eq!(seq(Failure::Transport(ErrorKind::Timeout), 3), [15, 30, 60]);
        assert_eq!(
            *seq(Failure::Transport(ErrorKind::Timeout), 20)
                .last()
                .unwrap(),
            900
        );
        assert_eq!(
            *seq(Failure::Transport(ErrorKind::Dns), 20).last().unwrap(),
            3600
        );
        assert_eq!(seq(Failure::TooMany(None), 2), [60, 120]);
        // Retry-After wins when longer, and is capped
        assert_eq!(seq(Failure::TooMany(Some(600)), 1), [600]);
        assert_eq!(seq(Failure::TooMany(Some(10)), 1), [60]);
        assert_eq!(seq(Failure::TooMany(Some(999_999)), 1), [3600]);
    }

    #[test]
    fn proxies_from_the_environment_hide_credentials() {
        // (reads the process environment; only checks the formatting helper's output shape)
        if let Some(p) = env_proxy() {
            assert!(!p.contains('@'), "{p}");
        }
    }

    /// A tiny HTTP server: answers each connection with the next canned response (`None` =
    /// accept and never answer).
    async fn fake_server(answers: Vec<Option<&'static str>>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            for a in answers {
                let Ok((mut sock, _)) = l.accept().await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                match a {
                    Some(resp) => {
                        let _ = sock.write_all(resp.as_bytes()).await;
                        let _ = sock.shutdown().await;
                    }
                    None => held.push(sock),
                }
            }
            // no more answers: close the port (later requests are refused)
            drop(l);
            tokio::time::sleep(Duration::from_secs(30)).await;
            drop(held);
        });
        format!("http://{addr}")
    }

    fn test_http(timeout_ms: u64) -> ReqwestGet {
        ReqwestGet::build(
            "freeLib-test",
            Duration::from_millis(timeout_ms),
            Duration::from_millis(timeout_ms),
            false,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn real_http_errors_are_classified() {
        // 429 with Retry-After, then a 503, then JSON
        let base = fake_server(vec![
            Some("HTTP/1.1 429 Too Many Requests\r\nRetry-After: 120\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            Some("HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy"),
            Some("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"),
            None,
        ])
        .await;
        let http = test_http(700);
        let r = http.get(format!("{base}/a")).await.unwrap();
        assert_eq!((r.status, r.retry_after), (429, Some(120)));
        let r = http.get(format!("{base}/b")).await.unwrap();
        assert_eq!((r.status, r.body.as_str()), (503, "busy"));
        let r = http.get(format!("{base}/c")).await.unwrap();
        assert_eq!((r.status, r.body.as_str()), (200, "{}"));
        // accepted, never answered: a timeout
        let e = http.get(format!("{base}/d")).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Timeout, "{e:?}");
        assert!(e.detail.contains("/d"), "the URL is in the chain: {e:?}");
        // nobody listening: a connection error with its cause
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let e = http
            .get(format!("http://127.0.0.1:{port}/x"))
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Connect, "{e:?}");
        assert!(e.detail.to_lowercase().contains("refused"), "{e:?}");
        // a name that cannot resolve
        let e = http
            .get("http://freelib-test.invalid/x".into())
            .await
            .unwrap_err();
        assert!(
            matches!(e.kind, ErrorKind::Dns | ErrorKind::Timeout),
            "{e:?}"
        );
    }

    #[tokio::test]
    async fn client_reports_errors_with_their_kind_and_pause() {
        let base = fake_server(vec![
            Some("HTTP/1.1 429 Too Many Requests\r\nRetry-After: 90\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            Some("HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
        ])
        .await;
        let ol = OpenLibrary::new(
            &base,
            Arc::new(test_http(2000)),
            Arc::new(Limiter::new(Duration::from_millis(1))),
        );
        let out = ol.lookup(&q("Dune", "Herbert")).await;
        assert_eq!(
            out,
            Outcome::Error("HTTP 429 (too many requests), Retry-After 90 s; pausing 90 s".into())
        );
        // (the pause is real: reset it instead of waiting)
        ol.limiter.succeeded();
        let out = ol.lookup(&q("Dune", "Herbert")).await;
        assert_eq!(
            out,
            Outcome::Error("HTTP 502 (server error); pausing 30 s".into())
        );
        ol.limiter.succeeded();
        // the server is gone: a connection error
        let out = ol.lookup(&q("Dune", "Herbert")).await;
        let Outcome::Error(e) = out else {
            panic!("{out:?}")
        };
        assert!(
            e.starts_with("connection error: ") && e.ends_with("; pausing 30 s"),
            "{e}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn phonetic_author_and_main_title_attempts() {
        // «Азимов» on Open Library is «Isaac Asimov»
        assert!(authors_match(&["Азимов".into()], &["Isaac Asimov".into()]));
        assert!(authors_match(&["Толстой".into()], &["Leo Tolstoy".into()]));
        assert!(!authors_match(&["Азимов".into()], &["Agaev".into()]));
        // a subtitle: the main title is tried as well
        let http = FakeHttp::new(vec![("/search.json", 200, EMPTY)]);
        let out = client(http.clone())
            .lookup(&q("Солярис: роман в двух частях", "Лем"))
            .await;
        assert_eq!(out, Outcome::NotFound);
        let urls = http.urls();
        assert!(
            urls.iter()
                .any(|u| u.contains("title=%D0%A1%D0%BE%D0%BB%D1%8F%D1%80%D0%B8%D1%81&")),
            "{urls:?}"
        );
        assert!(
            urls.iter().any(|u| u.contains("title=Solyaris&")),
            "{urls:?}"
        );
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
