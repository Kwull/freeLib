//! Shared application state.

use std::collections::HashMap;
use std::net::IpAddr;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use freelib_catalog::{Catalog, CatalogHandle, NameList};
use serde::Serialize;
use tokio::sync::{Semaphore, broadcast};

use crate::calibre::Calibre;
use crate::config::Config;
use crate::conv::{Converter, Fb2Conv};
use crate::db::{self, AppDb, LibraryRow, User};
use crate::error::{ApiError, ApiResult};
use crate::jobs::{Event, JobManager};

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

pub struct Inner {
    pub cfg: Config,
    pub db: AppDb,
    pub libs: RwLock<HashMap<i64, Arc<LibRuntime>>>,
    pub jobs: JobManager,
    pub open_mode: AtomicBool,
    /// Conversion worker pool (`FREELIB_WORKERS`).
    pub workers: Arc<Semaphore>,
    pub calibre: Option<Calibre>,
    pub conv: Arc<dyn Converter>,
    pub login_limiter: RateLimiter,
    /// Concurrent password verifications (Argon2 needs ~19 MiB each).
    pub verify_sem: Semaphore,
    /// Concurrent whole-book reads for previews (annotation / cover extraction).
    pub preview_sem: Semaphore,
    /// Bytes written to the cache since the last eviction pass, and whether one is running.
    pub cache_written: AtomicU64,
    pub cache_evicting: AtomicBool,
    /// token hash → (user, cached at)
    pub session_cache: Mutex<HashMap<String, (User, Instant)>>,
    /// Basic-auth credentials hash → (user, cached at) for OPDS.
    pub basic_cache: Mutex<HashMap<String, (User, Instant)>>,
    /// Background tasks stop when this is set.
    pub shutdown: AtomicBool,
    /// OpenID Connect sign-in, when configured.
    pub oidc: Option<Arc<crate::oidc::Provider>>,
    /// External (Open Library) ratings: cache, index and enrichment queue.
    pub ext: Arc<crate::extrating::ExtRatings>,
    /// MCP requests per API token (rate limiting).
    pub tokens: crate::tokens::Tokens,
    /// Encryption keys of secrets stored in app.db.
    pub secrets: Arc<crate::secrets::Secrets>,
    /// OAuth authorization server state (pending consents, codes, client metadata cache).
    pub oauth: crate::oauth::OAuth,
    /// Cover presence of books whose preview was extracted (best copy of a work).
    pub covers: crate::find::CoverHints,
}

impl AppState {
    pub fn new(
        cfg: Config,
        db: AppDb,
        calibre: Option<Calibre>,
        oidc: Option<crate::oidc::Provider>,
        ext: Arc<crate::extrating::ExtRatings>,
        secrets: Arc<crate::secrets::Secrets>,
    ) -> AppState {
        let (tx, _) = broadcast::channel(1024);
        let workers = Arc::new(Semaphore::new(cfg.workers.max(1)));
        let preview_permits = cfg.workers.max(1) * 2;
        let oauth = crate::oauth::OAuth::new(&cfg);
        AppState(Arc::new(Inner {
            cfg,
            db,
            libs: RwLock::new(HashMap::new()),
            jobs: JobManager::new(tx),
            open_mode: AtomicBool::new(false),
            workers,
            calibre,
            conv: Arc::new(Fb2Conv),
            login_limiter: RateLimiter::default(),
            verify_sem: Semaphore::new(2),
            preview_sem: Semaphore::new(preview_permits),
            cache_written: AtomicU64::new(0),
            cache_evicting: AtomicBool::new(false),
            session_cache: Mutex::new(HashMap::new()),
            basic_cache: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
            oidc: oidc.map(Arc::new),
            ext,
            tokens: crate::tokens::Tokens::default(),
            secrets,
            oauth,
            covers: crate::find::CoverHints::default(),
        }))
    }

    pub fn open_mode(&self) -> bool {
        self.open_mode.load(Ordering::Relaxed)
    }

    /// Whether cookies set for this request need `Secure` (HTTPS behind a proxy, or an
    /// `https://` `FREELIB_PUBLIC_URL`).
    pub fn secure_cookies(&self, headers: &axum::http::HeaderMap) -> bool {
        self.cfg.public_https() || crate::auth::is_https(headers)
    }

    /// Password sign-in is refused for `username` (`FREELIB_OIDC_DISABLE_PASSWORD`; the
    /// `FREELIB_ADMIN_USER` account keeps it while `FREELIB_ADMIN_PASSWORD` is set).
    pub fn password_login_refused(&self, username: &str) -> bool {
        let disabled = self.cfg.oidc.as_ref().is_some_and(|o| o.disable_password);
        disabled
            && !(self.cfg.admin_password.is_some()
                && username.trim().eq_ignore_ascii_case(&self.cfg.admin_user))
    }

    pub fn events(&self) -> &broadcast::Sender<Event> {
        &self.jobs.events
    }

    pub fn emit_library(&self, id: i64) {
        let _ = self.events().send(Event::library(id));
    }

    /// Runs `f` on the current catalog of `lib` in a blocking thread. When the catalog was
    /// replaced by a re-import while `f` needed a new connection (`stale`), retries once with
    /// the fresh catalog.
    pub async fn catalog_call<R, F>(&self, lib: i64, f: F) -> ApiResult<R>
    where
        R: Send + 'static,
        F: Fn(&Arc<Catalog>) -> ApiResult<R> + Send + 'static,
    {
        let rt = self.lib(lib)?;
        tokio::task::spawn_blocking(move || {
            let cat = rt
                .handle
                .get()
                .ok_or_else(|| ApiError::not_found("library is not imported yet"))?;
            match f(&cat) {
                Err(e) if e.code == crate::error::STALE => {
                    let fresh = match rt.handle.get() {
                        Some(c) if !Arc::ptr_eq(&c, &cat) => c,
                        _ => rt.handle.reload()?,
                    };
                    f(&fresh)
                }
                r => r,
            }
        })
        .await?
    }

    pub fn lib(&self, id: i64) -> ApiResult<Arc<LibRuntime>> {
        self.libs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
            .ok_or_else(|| ApiError::not_found("library not found"))
    }

    /// The runtime of library `id` and its current catalog (404 when not imported yet).
    pub fn catalog(&self, id: i64) -> ApiResult<(Arc<LibRuntime>, Arc<Catalog>)> {
        let rt = self.lib(id)?;
        let cat = rt
            .handle
            .get()
            .ok_or_else(|| ApiError::not_found("library is not imported yet"))?;
        Ok((rt, cat))
    }

    pub fn add_lib(&self, id: i64) -> Arc<LibRuntime> {
        let rt = Arc::new(LibRuntime::new(
            id,
            self.cfg.data_dir.join(format!("lib_{id}.db")),
        ));
        self.libs
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, rt.clone());
        rt
    }

    pub fn remove_lib(&self, id: i64) -> Option<Arc<LibRuntime>> {
        self.libs
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
    }

    pub fn invalidate_sessions(&self) {
        self.tokens.invalidate();
        self.session_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.basic_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub fn cache_dir(&self, kind: &str, lib: i64) -> PathBuf {
        self.cfg.cache_dir.join(kind).join(lib.to_string())
    }

    /// Builds the API `Library` for `user` (blocking: may compute catalog attributes).
    pub fn library_dto(&self, row: &LibraryRow, user_id: i64) -> LibraryDto {
        let mut d = self.library_base_dto(row);
        d.new_since_last_visit = self.new_since_last_visit(row.id, user_id);
        d
    }

    /// Books added since the previous visit of `user_id` (blocking on a cache miss).
    pub fn new_since_last_visit(&self, lib: i64, user_id: i64) -> i64 {
        let Some(rt) = self.lib(lib).ok() else {
            return 0;
        };
        let Some(cat) = rt.handle.get() else {
            return 0;
        };
        let Some(after) = self.visit_baseline(user_id) else {
            return 0;
        };
        rt.count_newer(&cat, &after)
    }

    /// `newSinceLastVisit` from the cache only (`None` on a miss).
    pub fn new_since_cached(&self, lib: i64, user_id: i64) -> Option<i64> {
        let rt = self.lib(lib).ok()?;
        let cat = rt.handle.get()?;
        let Some(after) = self.visit_baseline(user_id) else {
            return Some(0);
        };
        rt.cached_newer(cat.catalog_version(), &after)
    }

    /// The API `Library` without the per-user `newSinceLastVisit` (0).
    pub fn library_base_dto(&self, row: &LibraryRow) -> LibraryDto {
        let rt = self.lib(row.id).ok();
        let cat = rt.as_ref().and_then(|r| r.handle.get());
        let status = rt.as_ref().map(|r| r.status()).unwrap_or_default();
        let new_since = 0;
        let st = cat.as_ref().map(|c| c.stats().clone()).unwrap_or_default();
        LibraryDto {
            id: row.id,
            name: row.name.clone(),
            path: row.path.clone(),
            inpx: row.inpx.clone(),
            first_author_only: row.first_author_only,
            skip_deleted: row.skip_deleted,
            is_default: row.is_default,
            book_count: st.live_book_count,
            author_count: st.author_count,
            series_count: st.series_count,
            imported_at: st.imported_at,
            catalog_version: st.catalog_version,
            new_since_last_visit: new_since,
            status,
            opds_url: format!("/opds/{}", row.id),
            external_ratings: self.ext.progress(row.id),
        }
    }

    /// `newSinceLastVisit` counts books whose date (a day, no time) is on or after the server's
    /// local date of the previous visit, i.e. `date > <that day - 1>`. Returns that bound.
    fn visit_baseline(&self, user_id: i64) -> Option<String> {
        let prev = {
            let c = self.db.lock();
            db::prev_visit(&c, user_id).ok().flatten()?
        };
        let secs = crate::util::parse_rfc3339(&prev)?;
        Some(crate::util::local_date_at(secs - 86_400))
    }

    /// The base DTO of library `id` (blocking).
    pub fn library_base_dto_by_id(&self, id: i64) -> ApiResult<LibraryDto> {
        let row = {
            let c = self.db.lock();
            db::get_library(&c, id)?
        };
        let row = row.ok_or_else(|| ApiError::not_found("library not found"))?;
        Ok(self.library_base_dto(&row))
    }

    pub async fn library_dto_async(&self, id: i64, user_id: i64) -> ApiResult<LibraryDto> {
        let st = self.clone();
        tokio::task::spawn_blocking(move || {
            let row = {
                let c = st.db.lock();
                db::get_library(&c, id)?
            };
            let row = row.ok_or_else(|| ApiError::not_found("library not found"))?;
            Ok(st.library_dto(&row, user_id))
        })
        .await?
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDto {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub inpx: Option<String>,
    pub first_author_only: bool,
    pub skip_deleted: bool,
    pub is_default: bool,
    pub book_count: i64,
    pub author_count: i64,
    pub series_count: i64,
    pub imported_at: Option<String>,
    pub catalog_version: i64,
    pub new_since_last_visit: i64,
    pub status: LibraryStatus,
    pub opds_url: String,
    /// Open Library lookups of this library's books.
    pub external_ratings: crate::extrating::Progress,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryStatus {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Why an import runs when the server started it by itself: `"upgrade"` (the catalog was
    /// written by another schema version and is rebuilt; the library cannot be browsed until
    /// it finishes) or `"autoimport"` (`FREELIB_AUTOIMPORT`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Default for LibraryStatus {
    fn default() -> Self {
        LibraryStatus {
            state: "idle".into(),
            progress: None,
            message: None,
            reason: None,
        }
    }
}

/// A precomputed JSON body with lazily compressed variants.
pub struct CachedBody {
    pub version: i64,
    pub etag: String,
    pub json: Bytes,
    br: OnceLock<Bytes>,
    gzip: OnceLock<Bytes>,
}

impl CachedBody {
    pub fn new(version: i64, etag: String, json: Vec<u8>) -> CachedBody {
        CachedBody {
            version,
            etag,
            json: Bytes::from(json),
            br: OnceLock::new(),
            gzip: OnceLock::new(),
        }
    }

    /// Blocking (compresses on first use).
    pub fn encoded(&self, enc: &str) -> Bytes {
        match enc {
            "br" => self
                .br
                .get_or_init(|| Bytes::from(crate::compress::brotli(&self.json)))
                .clone(),
            "gzip" => self
                .gzip
                .get_or_init(|| Bytes::from(crate::compress::gzip(&self.json)))
                .clone(),
            _ => self.json.clone(),
        }
    }
}

pub struct ImportRun {
    pub job_id: String,
}

/// Runtime state of one library.
pub struct LibRuntime {
    pub id: i64,
    pub handle: CatalogHandle,
    status: Mutex<LibraryStatus>,
    pub import: Mutex<Option<ImportRun>>,
    /// "authors" / "series" → cached body for the current catalog version.
    pub lists: Mutex<HashMap<&'static str, Arc<CachedBody>>>,
    names: Mutex<HashMap<&'static str, (i64, Arc<NameList>)>>,
    /// Serialises building of the lists (a request and the warm-up must not both build them).
    pub build_lock: Mutex<()>,
    /// Set (under the `import` lock) when the library is deleted: no new import starts and an
    /// import that is still finishing discards its result.
    pub deleted: AtomicBool,
    /// (catalog version, date bound) → `count_newer_than` result.
    newer: Mutex<HashMap<(i64, String), i64>>,
}

impl LibRuntime {
    pub fn new(id: i64, path: PathBuf) -> LibRuntime {
        LibRuntime {
            id,
            handle: CatalogHandle::new(path),
            status: Mutex::new(LibraryStatus::default()),
            import: Mutex::new(None),
            lists: Mutex::new(HashMap::new()),
            names: Mutex::new(HashMap::new()),
            build_lock: Mutex::new(()),
            deleted: AtomicBool::new(false),
            newer: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_deleted(&self) -> bool {
        self.deleted.load(Ordering::SeqCst)
    }

    fn cached_newer(&self, version: i64, after: &str) -> Option<i64> {
        self.newer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(version, after.to_string()))
            .copied()
    }

    /// Books of `cat` dated after `after`, cached per catalog version (blocking on a miss).
    pub fn count_newer(&self, cat: &Catalog, after: &str) -> i64 {
        let v = cat.catalog_version();
        if let Some(n) = self.cached_newer(v, after) {
            return n;
        }
        let n = cat.count_newer_than(after).unwrap_or(0);
        let mut g = self.newer.lock().unwrap_or_else(|e| e.into_inner());
        g.retain(|(ver, _), _| *ver == v);
        if g.len() > 1000 {
            g.clear();
        }
        g.insert((v, after.to_string()), n);
        n
    }

    pub fn status(&self) -> LibraryStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_status(&self, s: LibraryStatus) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }

    pub fn cached_list(&self, kind: &'static str, version: i64) -> Option<Arc<CachedBody>> {
        self.lists
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(kind)
            .filter(|c| c.version == version)
            .cloned()
    }

    pub fn put_list(&self, kind: &'static str, body: Arc<CachedBody>) {
        self.lists
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(kind, body);
    }

    pub fn clear_lists(&self) {
        self.lists.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.names.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Authors (`"authors"`) or series list of `cat`, cached per catalog version. Blocking.
    pub fn name_list(
        &self,
        cat: &Catalog,
        kind: &'static str,
    ) -> Result<Arc<NameList>, freelib_catalog::CatalogError> {
        let v = cat.catalog_version();
        if let Some((ver, l)) = self
            .names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(kind)
            && *ver == v
        {
            return Ok(l.clone());
        }
        let l = Arc::new(if kind == "authors" {
            cat.authors()?
        } else {
            cat.series_list()?
        });
        self.names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(kind, (v, l.clone()));
        Ok(l)
    }
}

/// What a login attempt is throttled by: the client address (IPv6 per /64) and the user name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LimitKey {
    Ip(IpAddr),
    User(String),
}

impl LimitKey {
    /// IPv4 as is (also when IPv4-mapped), IPv6 truncated to its /64 prefix: one host usually
    /// owns a whole /64, so rotating addresses inside it must not reset the backoff.
    pub fn ip(ip: IpAddr) -> LimitKey {
        LimitKey::Ip(match ip {
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => {
                    let s = v6.segments();
                    IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
                }
            },
            v4 => v4,
        })
    }

    /// Case-insensitive, like user names.
    pub fn user(name: &str) -> LimitKey {
        LimitKey::User(name.trim().to_lowercase())
    }
}

#[derive(Debug)]
struct Slot {
    failures: u32,
    inflight: u32,
    until: Instant,
}

/// Login backoff, per client address and per user name independently: after
/// [`RateLimiter::FREE_FAILURES`] failures each further failure doubles the wait (max 5 min).
/// Attempts in flight count as failures until they finish, so parallel guesses cannot race
/// past the limit, and at most [`RateLimiter::MAX_INFLIGHT`] attempts per key run at once.
#[derive(Default)]
pub struct RateLimiter {
    map: Mutex<HashMap<LimitKey, Slot>>,
}

impl RateLimiter {
    pub const FREE_FAILURES: u32 = 5;
    pub const MAX_INFLIGHT: u32 = 2;
    const MAX_WAIT: u64 = 300;

    /// Registers an attempt for all `keys`; `Err(seconds to wait)` when any of them is blocked.
    /// Every `Ok` must be followed by [`finish`](Self::finish) with the same keys.
    pub fn begin(&self, keys: &[LimitKey]) -> Result<(), u64> {
        let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        for k in keys {
            if let Some(s) = g.get(k) {
                if s.until > now {
                    return Err((s.until - now).as_secs().max(1));
                }
                if s.inflight >= Self::MAX_INFLIGHT
                    || (s.inflight > 0 && s.failures + s.inflight >= Self::FREE_FAILURES)
                {
                    return Err(1);
                }
            }
        }
        if g.len() > 10_000 {
            g.retain(|_, s| s.inflight > 0 || s.until + Duration::from_secs(3600) > now);
        }
        for k in keys {
            g.entry(k.clone())
                .or_insert(Slot {
                    failures: 0,
                    inflight: 0,
                    until: now,
                })
                .inflight += 1;
        }
        Ok(())
    }

    /// Ends an attempt started with [`begin`](Self::begin).
    pub fn finish(&self, keys: &[LimitKey], success: bool) {
        let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        for k in keys {
            let Some(s) = g.get_mut(k) else { continue };
            s.inflight = s.inflight.saturating_sub(1);
            if success {
                s.failures = 0;
                s.until = now;
            } else {
                s.failures += 1;
                if s.failures >= Self::FREE_FAILURES {
                    let secs = 1u64 << (s.failures - Self::FREE_FAILURES).min(9);
                    s.until = now + Duration::from_secs(secs.min(Self::MAX_WAIT));
                }
            }
            if s.inflight == 0 && s.failures == 0 {
                g.remove(k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_per_key() {
        let l = RateLimiter::default();
        let a = [
            LimitKey::ip("10.0.0.1".parse().unwrap()),
            LimitKey::user("Bob"),
        ];
        for _ in 0..RateLimiter::FREE_FAILURES {
            l.begin(&a).unwrap();
            l.finish(&a, false);
        }
        assert!(l.begin(&a).is_err(), "blocked after 5 failures");
        // same user from another address: blocked by the user key
        let b = [
            LimitKey::ip("10.0.0.2".parse().unwrap()),
            LimitKey::user("BOB"),
        ];
        assert!(l.begin(&b).is_err());
        // other user from another address: fine
        let c = [
            LimitKey::ip("10.0.0.2".parse().unwrap()),
            LimitKey::user("alice"),
        ];
        l.begin(&c).unwrap();
        l.finish(&c, true);
    }

    #[test]
    fn inflight_counts() {
        let l = RateLimiter::default();
        let k = [LimitKey::user("x")];
        l.begin(&k).unwrap();
        l.begin(&k).unwrap();
        assert!(l.begin(&k).is_err(), "at most two attempts in flight");
        l.finish(&k, false);
        l.finish(&k, false);
        l.begin(&k).unwrap();
        l.finish(&k, true);
        assert!(l.map.lock().unwrap().is_empty());
    }

    #[test]
    fn ipv6_buckets() {
        let a = LimitKey::ip("2001:db8:1:2:aaaa::1".parse().unwrap());
        let b = LimitKey::ip("2001:db8:1:2:bbbb::7".parse().unwrap());
        let c = LimitKey::ip("2001:db8:1:3::1".parse().unwrap());
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(
            LimitKey::ip("::ffff:10.1.2.3".parse().unwrap()),
            LimitKey::ip("10.1.2.3".parse().unwrap())
        );
    }
}
