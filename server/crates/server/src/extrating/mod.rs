//! External ratings (Open Library): cache, in-memory index and the background enrichment
//! worker.
//!
//! * **Cache**: `ratings.db` in the data directory (separate from `app.db`: written about once a
//!   second by the worker, safe to delete). One row per (library, `book_key`, source) with
//!   `status` `found` / `not_found` / `error`, average, vote count, Open Library work key and
//!   fetch time. Found ratings are refreshed after 90 days, misses after 180 days, errors
//!   retried after a day.
//! * **Memory**: per library, `book_key → (average × 100, votes)` of the found ratings (for
//!   book lists), plus — built on first use per catalog version — dense arrays indexed by book
//!   id for sorting and filtering whole genres (see [`Dense`]).
//! * **Worker**: one request at a time, ≥ 1 s apart ([`openlibrary::Limiter`]), backing off
//!   after errors. Priorities: (1) books the user opened, shelved, rated or sent, (2) books of
//!   authors and series the user browses, (3) the rest of every library, id by id. Page loads
//!   never wait for it: they only enqueue.
//! * With the admin setting `externalRatings.enabled` off nothing is sent to Open Library.

pub mod openlibrary;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use freelib_catalog::{Catalog, RatingSource};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use tokio::sync::Notify;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::{rfc3339_at, unix_now};
use openlibrary::{HttpGet, Limiter, OpenLibrary, Outcome, Query};

/// Source name stored with every row.
pub const SOURCE: &str = "openlibrary";
/// Refresh found ratings after this many seconds (90 days).
pub const REFRESH_FOUND: i64 = 90 * 86_400;
/// Retry books Open Library did not know after 180 days.
pub const REFRESH_NOT_FOUND: i64 = 180 * 86_400;
/// Retry failed lookups after a day.
pub const RETRY_ERROR: i64 = 86_400;
/// Queue bounds (oldest entries dropped).
const MAX_P1: usize = 2_000;
const MAX_P2: usize = 10_000;
/// Books enqueued per browsed author / series page.
pub const BROWSE_ENQUEUE: usize = 300;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS ext_rating (
  library_id INTEGER NOT NULL, book_key TEXT NOT NULL, source TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('found','not_found','error')),
  average REAL, count INTEGER NOT NULL DEFAULT 0, work_key TEXT,
  fetched_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, message TEXT,
  PRIMARY KEY (library_id, book_key, source)) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS sweep (library_id INTEGER PRIMARY KEY, next_id INTEGER NOT NULL, done_at INTEGER);
"#;

/// Whether a cached row is still good at `now`.
pub fn is_fresh(status: &str, fetched_at: i64, now: i64) -> bool {
    let age = now - fetched_at;
    match status {
        "found" => age < REFRESH_FOUND,
        "not_found" => age < REFRESH_NOT_FOUND,
        _ => age < RETRY_ERROR,
    }
}

/// A cached lookup (API `extRatingInfo`, MCP `get_external_rating`).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtEntry {
    pub source: &'static str,
    /// `found`, `not_found` or `error`.
    pub status: String,
    pub average: Option<f64>,
    pub count: u32,
    pub work_key: Option<String>,
    /// Link to the work on Open Library.
    pub url: Option<String>,
    pub fetched_at: String,
    #[serde(skip)]
    pub fetched_unix: i64,
}

/// Found ratings of one catalog version as arrays indexed by book id: average × 100 and votes
/// (0 = none). 6 bytes per book (≈ 3.6 MB for 600k books).
pub struct Dense {
    pub version: i64,
    avg: Vec<u16>,
    votes: Vec<u32>,
}

impl Dense {
    pub fn get(&self, id: i64) -> Option<(u16, u32)> {
        let i = id as usize;
        match (self.avg.get(i), self.votes.get(i)) {
            (Some(&a), Some(&v)) if v > 0 => Some((a, v)),
            _ => None,
        }
    }

    fn set(&mut self, id: i64, avg: u16, votes: u32) {
        let i = id as usize;
        if i < self.avg.len() {
            self.avg[i] = avg;
            self.votes[i] = votes;
        }
    }
}

/// Ratings of one request: the user's own (by book id) and the external ones.
pub struct Ratings<'a> {
    pub my: HashMap<i64, u8>,
    pub ext: Option<std::sync::RwLockReadGuard<'a, Dense>>,
}

impl RatingSource for Ratings<'_> {
    fn my(&self, id: i64) -> u8 {
        self.my.get(&id).copied().unwrap_or(0)
    }
    fn ext(&self, id: i64) -> Option<(u16, u32)> {
        self.ext.as_ref().and_then(|d| d.get(id))
    }
}

#[derive(Default)]
struct LibMem {
    /// book_key → (average × 100, votes) of found ratings with votes.
    rated: HashMap<String, (u16, u32)>,
    looked_up: u64,
    found: u64,
    dense: Option<Arc<RwLock<Dense>>>,
}

#[derive(Default)]
struct Queue {
    p1: VecDeque<(i64, i64)>,
    p2: VecDeque<(i64, i64)>,
    set: HashSet<(i64, i64)>,
}

/// Why a book is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// The user opened, shelved, rated or sent it.
    User,
    /// It belongs to an author or series the user browses.
    Browse,
}

/// Progress for Settings and the libraries page.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// Books with a cached answer (found or not found).
    pub looked_up: u64,
    /// Books Open Library knows.
    pub found: u64,
    /// Found books that have ratings.
    pub rated: u64,
}

/// The external ratings service (one per server).
pub struct ExtRatings {
    db: Mutex<Connection>,
    mem: RwLock<HashMap<i64, LibMem>>,
    /// `externalRatings.enabled`.
    pub enabled: AtomicBool,
    client: OpenLibrary,
    queue: Mutex<Queue>,
    notify: Notify,
    last_error: Mutex<Option<String>>,
    /// HTTP requests sent (for tests and the status).
    pub requests: Arc<AtomicU64>,
}

/// Counts requests on their way to the real HTTP layer.
struct Counting {
    inner: Arc<dyn HttpGet>,
    n: Arc<AtomicU64>,
}

impl HttpGet for Counting {
    fn get(
        &self,
        url: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = openlibrary::HttpResult> + Send>> {
        self.n.fetch_add(1, Ordering::Relaxed);
        self.inner.get(url)
    }
}

fn text_array<S: AsRef<str>>(v: &[S]) -> Rc<Vec<Value>> {
    Rc::new(
        v.iter()
            .map(|s| Value::Text(s.as_ref().to_string()))
            .collect(),
    )
}

fn centi(avg: Option<f64>) -> u16 {
    avg.map(|a| (a.clamp(0.0, 5.0) * 100.0).round() as u16)
        .unwrap_or(0)
}

impl ExtRatings {
    /// Opens (creating) `ratings.db` and loads the found ratings into memory.
    pub fn open(
        path: &Path,
        base_url: &str,
        http: Arc<dyn HttpGet>,
        interval: Duration,
    ) -> anyhow::Result<ExtRatings> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        rusqlite::vtab::array::load_module(&conn)?;
        let mut mem: HashMap<i64, LibMem> = HashMap::new();
        {
            let mut st = conn
                .prepare("SELECT library_id, book_key, status, average, count FROM ext_rating")?;
            let mut q = st.query([])?;
            while let Some(r) = q.next()? {
                let lib: i64 = r.get(0)?;
                let status = r.get_ref(2)?.as_str().unwrap_or("");
                let m = mem.entry(lib).or_default();
                match status {
                    "found" => {
                        m.looked_up += 1;
                        m.found += 1;
                        let count: i64 = r.get(4)?;
                        let avg: Option<f64> = r.get(3)?;
                        if count > 0 && avg.is_some() {
                            m.rated.insert(r.get(1)?, (centi(avg), count as u32));
                        }
                    }
                    "not_found" => m.looked_up += 1,
                    _ => {}
                }
            }
        }
        let requests = Arc::new(AtomicU64::new(0));
        let http: Arc<dyn HttpGet> = Arc::new(Counting {
            inner: http,
            n: requests.clone(),
        });
        let limiter = Arc::new(Limiter::new(interval));
        Ok(ExtRatings {
            db: Mutex::new(conn),
            mem: RwLock::new(mem),
            enabled: AtomicBool::new(true),
            client: OpenLibrary::new(base_url, http, limiter),
            queue: Mutex::new(Queue::default()),
            notify: Notify::new(),
            last_error: Mutex::new(None),
            requests,
        })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Wakes the worker (a library was imported).
    pub fn wake(&self) {
        self.notify.notify_one();
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
        self.notify.notify_one();
    }

    /// (average, votes) of a book with ratings, from memory.
    pub fn get(&self, lib: i64, key: &str) -> Option<(f64, u32)> {
        let g = self.mem.read().unwrap_or_else(|e| e.into_inner());
        g.get(&lib)
            .and_then(|m| m.rated.get(key))
            .map(|(a, v)| (*a as f64 / 100.0, *v))
    }

    /// The cached row of a book.
    pub fn entry(&self, lib: i64, key: &str) -> Option<ExtEntry> {
        let c = self.db();
        c.query_row(
            "SELECT status, average, count, work_key, fetched_at FROM ext_rating \
             WHERE library_id=?1 AND book_key=?2 AND source=?3",
            params![lib, key, SOURCE],
            |r| {
                let work_key: Option<String> = r.get(3)?;
                let fetched: i64 = r.get(4)?;
                Ok(ExtEntry {
                    source: SOURCE,
                    status: r.get(0)?,
                    average: r.get(1)?,
                    count: r.get::<_, i64>(2)? as u32,
                    url: work_key
                        .as_ref()
                        .map(|k| format!("https://openlibrary.org{k}")),
                    work_key,
                    fetched_at: rfc3339_at(fetched),
                    fetched_unix: fetched,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    }

    /// Progress of one library.
    pub fn progress(&self, lib: i64) -> Progress {
        let g = self.mem.read().unwrap_or_else(|e| e.into_inner());
        g.get(&lib)
            .map(|m| Progress {
                looked_up: m.looked_up,
                found: m.found,
                rated: m.rated.len() as u64,
            })
            .unwrap_or_default()
    }

    /// Queued books (priorities 1 and 2).
    pub fn queued(&self) -> usize {
        let q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.p1.len() + q.p2.len()
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Seconds until requests resume after errors (0 = not paused).
    pub fn paused_for(&self) -> u64 {
        self.client
            .limiter
            .backoff_until()
            .map(|t| {
                t.saturating_duration_since(tokio::time::Instant::now())
                    .as_secs()
            })
            .unwrap_or(0)
    }

    /// Forgets a deleted library.
    pub fn remove_library(&self, lib: i64) {
        self.mem
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&lib);
        let c = self.db();
        let _ = c.execute("DELETE FROM ext_rating WHERE library_id=?1", [lib]);
        let _ = c.execute("DELETE FROM sweep WHERE library_id=?1", [lib]);
    }

    /// Adds books to the lookup queue (never blocks on the network).
    pub fn enqueue(&self, prio: Priority, lib: i64, ids: &[i64]) {
        if !self.is_enabled() || ids.is_empty() {
            return;
        }
        {
            let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            for &id in ids {
                let k = (lib, id);
                match prio {
                    Priority::User => {
                        if q.set.contains(&k) {
                            // promote from the browse queue
                            if let Some(i) = q.p2.iter().position(|x| *x == k) {
                                q.p2.remove(i);
                            } else {
                                continue;
                            }
                        }
                        q.set.insert(k);
                        q.p1.push_back(k);
                        if q.p1.len() > MAX_P1
                            && let Some(old) = q.p1.pop_front()
                        {
                            q.set.remove(&old);
                        }
                    }
                    Priority::Browse => {
                        if !q.set.insert(k) {
                            continue;
                        }
                        q.p2.push_back(k);
                        if q.p2.len() > MAX_P2
                            && let Some(old) = q.p2.pop_front()
                        {
                            q.set.remove(&old);
                        }
                    }
                }
            }
        }
        self.notify.notify_one();
    }

    fn pop_queued(&self) -> Option<(i64, i64)> {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        let k = q.p1.pop_front().or_else(|| q.p2.pop_front())?;
        q.set.remove(&k);
        Some(k)
    }

    /// Keys among `keys` whose cached row is still fresh.
    fn fresh_keys(&self, lib: i64, keys: &[String], now: i64) -> HashSet<String> {
        let c = self.db();
        let Ok(mut st) = c.prepare_cached(
            "SELECT book_key, status, fetched_at FROM ext_rating \
             WHERE library_id=?1 AND source=?2 AND book_key IN rarray(?3)",
        ) else {
            return HashSet::new();
        };
        let rows = st.query_map(params![lib, SOURCE, text_array(keys)], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        });
        let mut out = HashSet::new();
        if let Ok(rows) = rows {
            for (k, s, t) in rows.flatten() {
                if is_fresh(&s, t, now) {
                    out.insert(k);
                }
            }
        }
        out
    }

    /// Dense ratings of the catalog `cat` of `lib`, built on first use (blocking).
    pub fn dense(&self, lib: i64, cat: &Catalog) -> Option<Arc<RwLock<Dense>>> {
        let version = cat.catalog_version();
        {
            let g = self.mem.read().unwrap_or_else(|e| e.into_inner());
            if let Some(d) = g.get(&lib).and_then(|m| m.dense.clone())
                && d.read().unwrap_or_else(|e| e.into_inner()).version == version
            {
                return Some(d);
            }
        }
        let rated: Vec<(String, (u16, u32))> = {
            let g = self.mem.read().unwrap_or_else(|e| e.into_inner());
            g.get(&lib)
                .map(|m| m.rated.iter().map(|(k, v)| (k.clone(), *v)).collect())
                .unwrap_or_default()
        };
        let n = cat.attrs().ok()?.len();
        let mut d = Dense {
            version,
            avg: vec![0; n],
            votes: vec![0; n],
        };
        let by_key: HashMap<&str, (u16, u32)> =
            rated.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        for chunk in rated.chunks(20_000) {
            let keys: Vec<String> = chunk.iter().map(|(k, _)| k.clone()).collect();
            if let Ok(ids) = cat.ids_by_keys(&keys) {
                for (k, id) in ids {
                    if let Some((a, v)) = by_key.get(k.as_str()) {
                        d.set(id, *a, *v);
                    }
                }
            }
        }
        let d = Arc::new(RwLock::new(d));
        let mut g = self.mem.write().unwrap_or_else(|e| e.into_inner());
        g.entry(lib).or_default().dense = Some(d.clone());
        Some(d)
    }

    /// Stores a lookup result and updates the memory index.
    fn store(&self, lib: i64, key: &str, id: i64, version: i64, out: &Outcome) {
        let now = unix_now();
        let (status, avg, count, work, msg) = match out {
            Outcome::Found {
                work_key,
                average,
                count,
            } => ("found", *average, *count, Some(work_key.clone()), None),
            Outcome::NotFound => ("not_found", None, 0, None, None),
            Outcome::Error(e) => ("error", None, 0, None, Some(e.clone())),
        };
        let prev: Option<String> = {
            let c = self.db();
            let prev = c
                .query_row(
                    "SELECT status FROM ext_rating WHERE library_id=?1 AND book_key=?2 AND source=?3",
                    params![lib, key, SOURCE],
                    |r| r.get(0),
                )
                .optional()
                .ok()
                .flatten();
            if status == "error" && prev.as_deref().is_some_and(|p| p != "error") {
                // keep the last good answer; only its age says when to try again
                let _ = c.execute(
                    "UPDATE ext_rating SET attempts=attempts+1, message=?4 WHERE library_id=?1 AND book_key=?2 AND source=?3",
                    params![lib, key, SOURCE, msg],
                );
                return;
            }
            let _ = c.execute(
                "INSERT INTO ext_rating(library_id, book_key, source, status, average, count, work_key, fetched_at, attempts, message) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1,?9) \
                 ON CONFLICT(library_id, book_key, source) DO UPDATE SET status=excluded.status, average=excluded.average, \
                 count=excluded.count, work_key=excluded.work_key, fetched_at=excluded.fetched_at, \
                 attempts=CASE WHEN excluded.status='error' THEN ext_rating.attempts+1 ELSE 0 END, message=excluded.message",
                params![lib, key, SOURCE, status, avg, count, work, now, msg],
            );
            prev
        };
        let mut g = self.mem.write().unwrap_or_else(|e| e.into_inner());
        let m = g.entry(lib).or_default();
        let counted = |s: Option<&str>| matches!(s, Some("found") | Some("not_found"));
        if counted(prev.as_deref()) && !counted(Some(status)) {
            m.looked_up = m.looked_up.saturating_sub(1);
        } else if !counted(prev.as_deref()) && counted(Some(status)) {
            m.looked_up += 1;
        }
        if prev.as_deref() == Some("found") && status != "found" {
            m.found = m.found.saturating_sub(1);
        } else if prev.as_deref() != Some("found") && status == "found" {
            m.found += 1;
        }
        let rated = (status == "found" && count > 0 && avg.is_some()).then(|| (centi(avg), count));
        match rated {
            Some(v) => {
                m.rated.insert(key.to_string(), v);
            }
            None => {
                m.rated.remove(key);
            }
        }
        if let Some(d) = &m.dense {
            let mut d = d.write().unwrap_or_else(|e| e.into_inner());
            if d.version == version {
                let (a, v) = rated.unwrap_or((0, 0));
                d.set(id, a, v);
            }
        }
        drop(g);
        if let Outcome::Error(e) = out {
            *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e.clone());
        }
    }

    /// Looks one book up now (MCP `get_external_rating`), unless a fresh answer is cached.
    pub async fn lookup_now(&self, st: &AppState, lib: i64, id: i64) -> ApiResult<ExtEntry> {
        let book = load_query(st, lib, id).await?;
        if let Some(e) = self.entry(lib, &book.key)
            && is_fresh(&e.status, e.fetched_unix, unix_now())
        {
            return Ok(e);
        }
        if !self.is_enabled() {
            return Err(ApiError::forbidden(
                "external ratings are disabled by the administrator",
            ));
        }
        let out = tokio::time::timeout(Duration::from_secs(90), self.client.lookup(&book.query))
            .await
            .map_err(|_| ApiError::rate_limited("Open Library is busy; try again later"))?;
        self.store(lib, &book.key, id, book.version, &out);
        self.entry(lib, &book.key)
            .ok_or_else(|| ApiError::internal("rating not stored"))
    }

    /// Looks up one queued or swept book (the worker).
    async fn process(&self, st: &AppState, lib: i64, id: i64) {
        let Ok(book) = load_query(st, lib, id).await else {
            return;
        };
        if self
            .fresh_keys(lib, std::slice::from_ref(&book.key), unix_now())
            .contains(&book.key)
        {
            return;
        }
        let out = self.client.lookup(&book.query).await;
        self.store(lib, &book.key, id, book.version, &out);
    }

    /// The next book of the slow sweep over all libraries (blocking; `None` when every library
    /// is done for now).
    fn next_sweep(&self, st: &AppState) -> Option<(i64, i64)> {
        let now = unix_now();
        let mut libs: Vec<i64> = st
            .libs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .copied()
            .collect();
        libs.sort_unstable();
        for lib in libs {
            let Ok(rt) = st.lib(lib) else { continue };
            let Some(cat) = rt.handle.get() else { continue };
            let (mut next, done_at): (i64, Option<i64>) = {
                let c = self.db();
                c.query_row(
                    "SELECT next_id, done_at FROM sweep WHERE library_id=?1",
                    [lib],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .ok()
                .flatten()
                .unwrap_or((0, None))
            };
            if done_at.is_some_and(|d| now - d < 86_400) {
                continue; // a full pass ended less than a day ago
            }
            // at most a few batches per call, so a fully cached library does not stall the loop
            for _ in 0..20 {
                let batch: Vec<(i64, String)> = match cat.conn() {
                    Ok(conn) => conn
                        .prepare_cached(
                            "SELECT id, book_key FROM book WHERE id >= ?1 AND deleted=0 ORDER BY id LIMIT 500",
                        )
                        .and_then(|mut s| {
                            s.query_map([next], |r| Ok((r.get(0)?, r.get(1)?)))?
                                .collect::<rusqlite::Result<Vec<_>>>()
                        })
                        .unwrap_or_default(),
                    Err(_) => Vec::new(),
                };
                let c_set = |next: i64, done: Option<i64>| {
                    let c = self.db();
                    let _ = c.execute(
                        "INSERT INTO sweep(library_id, next_id, done_at) VALUES (?1,?2,?3) \
                         ON CONFLICT(library_id) DO UPDATE SET next_id=excluded.next_id, done_at=excluded.done_at",
                        params![lib, next, done],
                    );
                };
                if batch.is_empty() {
                    c_set(0, Some(now)); // pass complete: start over tomorrow (refreshes)
                    break;
                }
                let keys: Vec<String> = batch.iter().map(|(_, k)| k.clone()).collect();
                let fresh = self.fresh_keys(lib, &keys, now);
                if let Some((id, _)) = batch.iter().find(|(_, k)| !fresh.contains(k)) {
                    c_set(id + 1, None);
                    return Some((lib, *id));
                }
                next = batch.last().map(|(id, _)| id + 1).unwrap_or(next);
                c_set(next, None);
            }
        }
        None
    }

    /// The background loop (spawned at startup).
    pub async fn run(self: Arc<Self>, st: AppState) {
        loop {
            if st.shutdown.load(Ordering::Relaxed) {
                break;
            }
            if !self.is_enabled() {
                let _ = tokio::time::timeout(Duration::from_secs(60), self.notify.notified()).await;
                continue;
            }
            let item = match self.pop_queued() {
                Some(k) => Some(k),
                None => {
                    let (me, st2) = (self.clone(), st.clone());
                    tokio::task::spawn_blocking(move || me.next_sweep(&st2))
                        .await
                        .ok()
                        .flatten()
                }
            };
            match item {
                Some((lib, id)) => self.process(&st, lib, id).await,
                None => {
                    let _ = tokio::time::timeout(Duration::from_secs(600), self.notify.notified())
                        .await;
                }
            }
        }
    }
}

/// What the worker needs of a book.
struct BookQuery {
    key: String,
    version: i64,
    query: Query,
}

async fn load_query(st: &AppState, lib: i64, id: i64) -> ApiResult<BookQuery> {
    st.catalog_call(lib, move |cat| {
        let conn = cat.conn()?;
        let (key, title): (String, String) = conn
            .prepare_cached("SELECT book_key, title FROM book WHERE id=?1")?
            .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .ok_or_else(|| ApiError::not_found("book not found"))?;
        let surnames: Vec<String> = conn
            .prepare_cached(
                "SELECT a.last FROM book_author ba JOIN author a ON a.id=ba.author_id \
                 WHERE ba.book_id=?1 ORDER BY ba.pos LIMIT 3",
            )?
            .query_map([id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        // "Автор неизвестен" is no author to match on
        let surnames = surnames
            .into_iter()
            .filter(|s: &String| {
                let n = freelib_catalog::normalize(s);
                !n.is_empty() && n != "автор" && n != "unknown"
            })
            .collect();
        Ok(BookQuery {
            key,
            version: cat.catalog_version(),
            query: Query { title, surnames },
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> (tempfile::TempDir, ExtRatings) {
        let dir = tempfile::tempdir().unwrap();
        let http = openlibrary::tests::FakeHttp::new(vec![]);
        let e = ExtRatings::open(
            &dir.path().join("ratings.db"),
            "http://ol.test",
            http,
            Duration::from_millis(1),
        )
        .unwrap();
        (dir, e)
    }

    #[test]
    fn queue_priorities() {
        let (_d, e) = service();
        e.enqueue(Priority::Browse, 1, &[10, 11, 12]);
        e.enqueue(Priority::User, 1, &[20]);
        e.enqueue(Priority::User, 1, &[11]); // promoted from the browse queue
        e.enqueue(Priority::Browse, 1, &[20, 10]); // already queued: ignored
        assert_eq!(e.queued(), 4);
        let order: Vec<(i64, i64)> = std::iter::from_fn(|| e.pop_queued()).collect();
        assert_eq!(order, vec![(1, 20), (1, 11), (1, 10), (1, 12)]);
        // disabled: nothing is queued
        e.set_enabled(false);
        e.enqueue(Priority::User, 1, &[1, 2]);
        assert_eq!(e.queued(), 0);
        e.set_enabled(true);
        // bounded
        let many: Vec<i64> = (0..(MAX_P2 as i64 + 50)).collect();
        e.enqueue(Priority::Browse, 2, &many);
        assert_eq!(e.queued(), MAX_P2);
        assert_eq!(e.pop_queued(), Some((2, 50)), "oldest entries dropped");
    }

    #[test]
    fn store_updates_counters_and_memory() {
        let (_d, e) = service();
        let found = Outcome::Found {
            work_key: "/works/OL1W".into(),
            average: Some(4.26),
            count: 12,
        };
        e.store(1, "lib:1", 1, 7, &found);
        e.store(1, "lib:2", 2, 7, &Outcome::NotFound);
        e.store(1, "lib:3", 3, 7, &Outcome::Error("HTTP 503".into()));
        let p = e.progress(1);
        assert_eq!((p.looked_up, p.found, p.rated), (2, 1, 1));
        assert_eq!(e.get(1, "lib:1"), Some((4.26, 12)));
        assert_eq!(e.get(1, "lib:2"), None);
        let en = e.entry(1, "lib:1").unwrap();
        assert_eq!(en.status, "found");
        assert_eq!(
            en.url.as_deref(),
            Some("https://openlibrary.org/works/OL1W")
        );
        assert_eq!(e.entry(1, "lib:3").unwrap().status, "error");
        assert_eq!(e.last_error().as_deref(), Some("HTTP 503"));
        // a later error keeps the good answer
        e.store(1, "lib:1", 1, 7, &Outcome::Error("timeout".into()));
        assert_eq!(e.entry(1, "lib:1").unwrap().status, "found");
        assert_eq!(e.progress(1).found, 1);
        // re-lookup finds nothing any more
        e.store(1, "lib:1", 1, 7, &Outcome::NotFound);
        let p = e.progress(1);
        assert_eq!((p.looked_up, p.found, p.rated), (2, 0, 0));
        // fresh keys
        let keys = vec![
            "lib:1".to_string(),
            "lib:2".into(),
            "lib:3".into(),
            "lib:4".into(),
        ];
        let fresh = e.fresh_keys(1, &keys, unix_now());
        assert!(fresh.contains("lib:1") && fresh.contains("lib:2") && fresh.contains("lib:3"));
        assert!(!fresh.contains("lib:4"));
        // after 91 days the found / error rows are stale, not-found ones not yet
        let later = unix_now() + 91 * 86_400;
        let fresh = e.fresh_keys(1, &keys, later);
        assert!(fresh.contains("lib:2") && !fresh.contains("lib:3"));
        // reopening restores the counters from the file
        let path = _d.path().join("ratings.db");
        let e2 = ExtRatings::open(
            &path,
            "http://ol.test",
            openlibrary::tests::FakeHttp::new(vec![]),
            Duration::from_millis(1),
        )
        .unwrap();
        assert_eq!(e2.progress(1).looked_up, 2);
        e2.remove_library(1);
        assert_eq!(e2.progress(1).looked_up, 0);
        assert!(e2.entry(1, "lib:2").is_none());
    }

    #[test]
    fn freshness() {
        let now = 1_800_000_000;
        assert!(is_fresh("found", now - 89 * 86_400, now));
        assert!(!is_fresh("found", now - 91 * 86_400, now));
        assert!(is_fresh("not_found", now - 91 * 86_400, now));
        assert!(!is_fresh("error", now - 86_401, now));
        assert_eq!(centi(Some(4.126)), 413);
        assert_eq!(centi(None), 0);
    }
}
