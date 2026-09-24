//! Shared application state.

use std::collections::HashMap;
use std::net::IpAddr;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use freelib_catalog::{Catalog, CatalogHandle};
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
    /// token hash → (user, cached at)
    pub session_cache: Mutex<HashMap<String, (User, Instant)>>,
    /// Basic-auth credentials hash → (user, cached at) for OPDS.
    pub basic_cache: Mutex<HashMap<String, (User, Instant)>>,
    /// Previous visit per user (for `newSinceLastVisit`).
    pub prev_visit: Mutex<HashMap<i64, String>>,
    /// Background tasks stop when this is set.
    pub shutdown: AtomicBool,
}

impl AppState {
    pub fn new(cfg: Config, db: AppDb, calibre: Option<Calibre>) -> AppState {
        let (tx, _) = broadcast::channel(1024);
        let workers = Arc::new(Semaphore::new(cfg.workers.max(1)));
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
            session_cache: Mutex::new(HashMap::new()),
            basic_cache: Mutex::new(HashMap::new()),
            prev_visit: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
        }))
    }

    pub fn open_mode(&self) -> bool {
        self.open_mode.load(Ordering::Relaxed)
    }

    pub fn events(&self) -> &broadcast::Sender<Event> {
        &self.jobs.events
    }

    pub fn emit_library(&self, id: i64) {
        let _ = self.events().send(Event::Library { id });
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
        let cat = rt.handle.get().ok_or_else(|| ApiError::not_found("library is not imported yet"))?;
        Ok((rt, cat))
    }

    pub fn add_lib(&self, id: i64) -> Arc<LibRuntime> {
        let rt = Arc::new(LibRuntime::new(id, self.cfg.data_dir.join(format!("lib_{id}.db"))));
        self.libs.write().unwrap_or_else(|e| e.into_inner()).insert(id, rt.clone());
        rt
    }

    pub fn remove_lib(&self, id: i64) -> Option<Arc<LibRuntime>> {
        self.libs.write().unwrap_or_else(|e| e.into_inner()).remove(&id)
    }

    pub fn invalidate_sessions(&self) {
        self.session_cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.basic_cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn cache_dir(&self, kind: &str, lib: i64) -> PathBuf {
        self.cfg.cache_dir.join(kind).join(lib.to_string())
    }

    /// Builds the API `Library` for `user` (blocking: may compute catalog attributes).
    pub fn library_dto(&self, row: &LibraryRow, user_id: i64) -> LibraryDto {
        let rt = self.lib(row.id).ok();
        let cat = rt.as_ref().and_then(|r| r.handle.get());
        let status = rt.as_ref().map(|r| r.status()).unwrap_or_default();
        let since = self.visit_baseline(user_id);
        let new_since = match (&cat, since) {
            (Some(c), Some(s)) => c.count_newer_than(&s).unwrap_or(0),
            _ => 0,
        };
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
        }
    }

    /// Date (`YYYY-MM-DD`) of the user's previous visit.
    fn visit_baseline(&self, user_id: i64) -> Option<String> {
        if let Some(v) = self.prev_visit.lock().unwrap_or_else(|e| e.into_inner()).get(&user_id) {
            return Some(v.clone());
        }
        let c = self.db.lock();
        db::last_visit(&c, user_id).ok().flatten()
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
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryStatus {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Default for LibraryStatus {
    fn default() -> Self {
        LibraryStatus { state: "idle".into(), progress: None, message: None }
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
        CachedBody { version, etag, json: Bytes::from(json), br: OnceLock::new(), gzip: OnceLock::new() }
    }

    /// Blocking (compresses on first use).
    pub fn encoded(&self, enc: &str) -> Bytes {
        match enc {
            "br" => self.br.get_or_init(|| Bytes::from(crate::compress::brotli(&self.json))).clone(),
            "gzip" => self.gzip.get_or_init(|| Bytes::from(crate::compress::gzip(&self.json))).clone(),
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
}

impl LibRuntime {
    pub fn new(id: i64, path: PathBuf) -> LibRuntime {
        LibRuntime {
            id,
            handle: CatalogHandle::new(path),
            status: Mutex::new(LibraryStatus::default()),
            import: Mutex::new(None),
            lists: Mutex::new(HashMap::new()),
        }
    }

    pub fn status(&self) -> LibraryStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
        self.lists.lock().unwrap_or_else(|e| e.into_inner()).insert(kind, body);
    }

    pub fn clear_lists(&self) {
        self.lists.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// Per-IP login backoff: after 5 failures each further failure doubles the wait (max 5 min).
#[derive(Default)]
pub struct RateLimiter {
    map: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl RateLimiter {
    /// Seconds to wait before the next attempt is allowed.
    pub fn check(&self, ip: IpAddr) -> Option<u64> {
        let g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let (_, until) = g.get(&ip)?;
        let now = Instant::now();
        (*until > now).then(|| (*until - now).as_secs().max(1))
    }

    pub fn failure(&self, ip: IpAddr) {
        let mut g = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() > 10_000 {
            let now = Instant::now();
            g.retain(|_, (_, until)| *until + Duration::from_secs(3600) > now);
        }
        let e = g.entry(ip).or_insert((0, Instant::now()));
        e.0 += 1;
        if e.0 >= 5 {
            let secs = 1u64 << (e.0 - 5).min(9);
            e.1 = Instant::now() + Duration::from_secs(secs.min(300));
        }
    }

    pub fn success(&self, ip: IpAddr) {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).remove(&ip);
    }
}
