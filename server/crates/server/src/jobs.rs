//! Job list (imports, send/export/download) with SSE events.
//!
//! The list lives in memory (fast reads, SSE); send, export and download jobs are also written
//! through to `app.db` (`job`, `job_item`) by a background writer, so they survive restarts:
//! at startup queued jobs are resumed and jobs that were running are marked failed and
//! retryable ([`JobManager::load`]). Import jobs stay in memory only (an interrupted import is
//! restarted from the library state instead).
//!
//! A job carries per-book [`JobItem`]s (up to [`MAX_ITEMS`] books) with their delivery state:
//! `queued → converting → converted → sending → accepted` for e-mail (`retrying` while waiting
//! for an automatic retry), `saved` for folder exports, `ready` for downloads, `failed`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::db::{AppDb, User};
use crate::util::{now_rfc3339, random_id, unix_now};

pub const MAX_LOG: usize = 50;
/// Jobs kept per user in `GET /jobs`.
pub const MAX_LIST: usize = 50;
/// Jobs with more books than this report counts only (no per-book items).
pub const MAX_ITEMS: usize = 200;
/// Item updates of one job are sent to SSE clients at most this often.
const ITEM_EMIT_INTERVAL: Duration = Duration::from_millis(200);

/// Item states that mean "this book got where it should" (not redone by a retry).
pub const ITEM_OK: [&str; 3] = ["accepted", "saved", "ready"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobItem {
    pub book_id: i64,
    pub title: String,
    /// `queued`, `converting`, `converted`, `sending`, `retrying`, `accepted`, `saved`,
    /// `ready`, `failed`.
    pub state: String,
    /// The server's reply (`250 2.0.0 Ok: queued as …`), the error, or the retry plan.
    pub detail: String,
    /// Delivery attempts so far.
    pub attempts: u32,
    /// Size of the produced file in bytes.
    pub size: Option<u64>,
    /// Which mail of the job (1-based) carries this book.
    pub mail: Option<u32>,
}

impl JobItem {
    pub fn new(book_id: i64, title: &str) -> JobItem {
        JobItem {
            book_id,
            title: title.to_string(),
            state: "queued".into(),
            detail: String::new(),
            attempts: 0,
            size: None,
            mail: None,
        }
    }
    pub fn is_ok(&self) -> bool {
        ITEM_OK.contains(&self.state.as_str())
    }
}

/// Advice shown with a finished job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobHint {
    /// `kindle_approved_sender`
    pub code: String,
    /// The sender address to allow.
    pub from: String,
    pub url: String,
    /// English text (MCP, logs); the web app localises by `code`.
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub state: String,
    pub progress: f64,
    pub message: String,
    pub log: Vec<String>,
    pub download_url: Option<String>,
    pub created_at: String,
    pub finished_at: Option<String>,
    /// Per-book results (empty for imports and for jobs over [`MAX_ITEMS`] books).
    pub items: Vec<JobItem>,
    /// `POST /jobs/:id/retry` would redo the failed or interrupted books.
    pub retryable: bool,
    pub hint: Option<JobHint>,
}

impl Job {
    pub fn is_finished(&self) -> bool {
        matches!(self.state.as_str(), "done" | "failed" | "cancelled")
    }
}

/// A file produced by a job (`GET /jobs/:id/download`).
#[derive(Debug, Clone)]
pub struct JobFile {
    pub path: PathBuf,
    pub name: String,
    pub mime: String,
}

struct Entry {
    job: Job,
    owner: i64,
    cancel: Arc<AtomicBool>,
    file: Option<JobFile>,
    finished_unix: Option<i64>,
    /// Directory with files owned by this job (removed with the job).
    dir: Option<PathBuf>,
    /// What to redo on resume / retry (JSON of `sender::StoredRequest`); persisted jobs only.
    request: Option<String>,
    last_emit: Option<Instant>,
}

impl Entry {
    fn refresh(&mut self) {
        let j = &mut self.job;
        j.retryable = self.request.is_some()
            && (j.state == "failed"
                || (j.state == "done" && j.items.iter().any(|i| i.state == "failed")));
    }
}

/// The library DTO of one `library` event, built once by the first SSE subscriber that needs
/// it and shared by all others (only `newSinceLastVisit` differs per user).
pub type SharedDto = Arc<tokio::sync::OnceCell<Option<crate::state::LibraryDto>>>;

/// Server-sent events.
#[derive(Debug, Clone)]
pub enum Event {
    Job {
        owner: i64,
        job: Box<Job>,
    },
    Library {
        id: i64,
        dto: SharedDto,
    },
    /// Users were changed or deleted: SSE streams re-check their user.
    Users,
}

impl Event {
    pub fn library(id: i64) -> Event {
        Event::Library {
            id,
            dto: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    /// Whether `user` may see this event.
    pub fn visible_to(&self, user: &User) -> bool {
        match self {
            Event::Job { owner, job } => {
                *owner == user.id || (user.is_admin() && job.kind == "import")
            }
            Event::Library { .. } => true,
            Event::Users => false,
        }
    }
}

/// Kinds written to `app.db`.
fn persisted(kind: &str) -> bool {
    matches!(kind, "send" | "export" | "download")
}

struct JobRow {
    job: Job,
    owner: i64,
    request: Option<String>,
    file: Option<JobFile>,
    dir: Option<PathBuf>,
    finished_unix: Option<i64>,
}

enum Persist {
    Job(Box<JobRow>),
    Item(String, usize, JobItem),
    Items(String, Vec<JobItem>),
    Delete(Vec<String>),
    Flush(oneshot::Sender<()>),
}

/// A job to resume after a restart: `(id, owner, request JSON)`.
pub type Resume = (String, i64, String);

pub struct JobManager {
    entries: Mutex<Vec<Entry>>,
    pub events: broadcast::Sender<Event>,
    persist: OnceLock<mpsc::UnboundedSender<Persist>>,
}

impl JobManager {
    pub fn new(events: broadcast::Sender<Event>) -> JobManager {
        JobManager {
            entries: Mutex::new(Vec::new()),
            events,
            persist: OnceLock::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, owner: i64, job: Job) {
        let _ = self.events.send(Event::Job {
            owner,
            job: Box::new(job),
        });
    }

    fn send(&self, op: Persist) {
        if let Some(tx) = self.persist.get() {
            let _ = tx.send(op);
        }
    }

    fn persist_entry(&self, e: &Entry) {
        if persisted(&e.job.kind) {
            self.send(Persist::Job(Box::new(JobRow {
                job: e.job.clone(),
                owner: e.owner,
                request: e.request.clone(),
                file: e.file.clone(),
                dir: e.dir.clone(),
                finished_unix: e.finished_unix,
            })));
        }
    }

    /// Starts writing send/export/download jobs to `app.db` (a background task applies the
    /// changes in order, batched in transactions).
    pub fn attach_db(&self, db: AppDb) {
        let (tx, mut rx) = mpsc::unbounded_channel::<Persist>();
        if self.persist.set(tx).is_err() {
            return;
        }
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut ops = vec![first];
                while ops.len() < 1000 {
                    match rx.try_recv() {
                        Ok(op) => ops.push(op),
                        Err(_) => break,
                    }
                }
                let db = db.clone();
                let acks = tokio::task::spawn_blocking(move || {
                    let c = db.lock();
                    let mut acks = Vec::new();
                    if let Err(e) = write_ops(&c, ops, &mut acks) {
                        tracing::warn!("saving jobs failed: {e}");
                    }
                    acks
                })
                .await
                .unwrap_or_default();
                for a in acks {
                    let _ = a.send(());
                }
            }
        });
    }

    /// Waits until every change made so far is in `app.db`.
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        self.send(Persist::Flush(tx));
        if self.persist.get().is_some() {
            let _ = tokio::time::timeout(Duration::from_secs(10), rx).await;
        }
    }

    /// Loads the persisted jobs (call once at startup, before [`attach_db`](Self::attach_db)
    /// writes anything). Jobs that were running are marked failed and retryable ("interrupted
    /// by a restart"); queued jobs are returned so the caller resumes them. Jobs of users that
    /// no longer exist are dropped.
    pub fn load(&self, db: &AppDb) -> anyhow::Result<Vec<Resume>> {
        let c = db.lock();
        c.execute(
            "DELETE FROM job WHERE owner<>0 AND owner NOT IN (SELECT id FROM user)",
            [],
        )?;
        let mut st = c.prepare(
            "SELECT id, owner, kind, title, state, progress, message, log, request, hint, file_path, \
             file_name, file_mime, dir, created_at, finished_at, finished_unix FROM job ORDER BY created_at, rowid",
        )?;
        let rows = st.query_map([], |r| {
            let log: String = r.get(7)?;
            let hint: Option<String> = r.get(9)?;
            let file_path: Option<String> = r.get(10)?;
            let file = match (file_path, r.get::<_, Option<String>>(11)?) {
                (Some(p), Some(n)) => Some(JobFile {
                    path: PathBuf::from(p),
                    name: n,
                    mime: r
                        .get::<_, Option<String>>(12)?
                        .unwrap_or_else(|| "application/octet-stream".into()),
                }),
                _ => None,
            };
            let id: String = r.get(0)?;
            Ok(Entry {
                job: Job {
                    download_url: file.as_ref().map(|_| format!("/api/v1/jobs/{id}/download")),
                    id,
                    kind: r.get(2)?,
                    title: r.get(3)?,
                    state: r.get(4)?,
                    progress: r.get(5)?,
                    message: r.get(6)?,
                    log: serde_json::from_str(&log).unwrap_or_default(),
                    created_at: r.get(14)?,
                    finished_at: r.get(15)?,
                    items: Vec::new(),
                    retryable: false,
                    hint: hint.and_then(|h| serde_json::from_str(&h).ok()),
                },
                owner: r.get(1)?,
                cancel: Arc::new(AtomicBool::new(false)),
                file,
                finished_unix: r.get(16)?,
                dir: r.get::<_, Option<String>>(13)?.map(PathBuf::from),
                request: r.get(8)?,
                last_emit: None,
            })
        })?;
        let mut loaded: Vec<Entry> = rows.collect::<Result<_, _>>()?;
        let mut items_st = c.prepare(
            "SELECT book_id, title, state, detail, attempts, size, mail FROM job_item WHERE job_id=?1 ORDER BY pos",
        )?;
        let mut resume = Vec::new();
        let now = now_rfc3339();
        for e in &mut loaded {
            e.job.items = items_st
                .query_map([&e.job.id], |r| {
                    Ok(JobItem {
                        book_id: r.get(0)?,
                        title: r.get(1)?,
                        state: r.get(2)?,
                        detail: r.get(3)?,
                        attempts: r.get(4)?,
                        size: r.get::<_, Option<i64>>(5)?.map(|s| s as u64),
                        mail: r.get(6)?,
                    })
                })?
                .collect::<Result<_, _>>()?;
            match e.job.state.as_str() {
                "running" => {
                    e.job.state = "failed".into();
                    e.job.message = "Interrupted by a server restart; retry to finish".into();
                    e.job.finished_at = Some(now.clone());
                    e.finished_unix = Some(unix_now());
                    for it in &mut e.job.items {
                        if !it.is_ok() && it.state != "failed" {
                            it.state = "failed".into();
                            it.detail = "interrupted by a server restart".into();
                        }
                    }
                }
                "queued" => {
                    if let Some(r) = &e.request {
                        resume.push((e.job.id.clone(), e.owner, r.clone()));
                    } else {
                        e.job.state = "failed".into();
                        e.job.message = "Interrupted by a server restart".into();
                    }
                }
                _ => {}
            }
            e.refresh();
        }
        drop(items_st);
        drop(st);
        drop(c);
        let mut g = self.lock();
        for e in loaded {
            if g.iter().any(|x| x.job.id == e.job.id) {
                continue;
            }
            // write back the interrupted state (items too)
            self.persist_entry(&e);
            if persisted(&e.job.kind) {
                self.send(Persist::Items(e.job.id.clone(), e.job.items.clone()));
            }
            g.push(e);
        }
        Ok(resume)
    }

    /// Creates a queued job unless `owner` already has `max_active` queued or running jobs
    /// (`None` then); returns it and its cancellation flag.
    pub fn try_create(
        &self,
        kind: &str,
        title: &str,
        owner: i64,
        max_active: usize,
    ) -> Option<(Job, Arc<AtomicBool>)> {
        self.try_create_with(kind, title, owner, max_active, None, Vec::new())
    }

    /// [`try_create`](Self::try_create) with the stored request (for resume / retry) and the
    /// per-book items.
    pub fn try_create_with(
        &self,
        kind: &str,
        title: &str,
        owner: i64,
        max_active: usize,
        request: Option<String>,
        items: Vec<JobItem>,
    ) -> Option<(Job, Arc<AtomicBool>)> {
        {
            let g = self.lock();
            let active = g
                .iter()
                .filter(|e| e.owner == owner && !e.job.is_finished())
                .count();
            if active >= max_active {
                return None;
            }
        }
        Some(self.create_with(kind, title, owner, request, items))
    }

    /// Creates a queued job; returns it and its cancellation flag.
    pub fn create(&self, kind: &str, title: &str, owner: i64) -> (Job, Arc<AtomicBool>) {
        self.create_with(kind, title, owner, None, Vec::new())
    }

    fn create_with(
        &self,
        kind: &str,
        title: &str,
        owner: i64,
        request: Option<String>,
        items: Vec<JobItem>,
    ) -> (Job, Arc<AtomicBool>) {
        let job = Job {
            id: random_id(),
            kind: kind.into(),
            title: title.into(),
            state: "queued".into(),
            progress: 0.0,
            message: String::new(),
            log: Vec::new(),
            download_url: None,
            created_at: now_rfc3339(),
            finished_at: None,
            items,
            retryable: false,
            hint: None,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let e = Entry {
            job: job.clone(),
            owner,
            cancel: cancel.clone(),
            file: None,
            finished_unix: None,
            dir: None,
            request,
            last_emit: Some(Instant::now()),
        };
        self.persist_entry(&e);
        if persisted(kind) && !job.items.is_empty() {
            self.send(Persist::Items(job.id.clone(), job.items.clone()));
        }
        self.lock().push(e);
        self.emit(owner, job.clone());
        (job, cancel)
    }

    /// Applies `f` to a job (ignored once finished) and emits the new state.
    pub fn update(&self, id: &str, f: impl FnOnce(&mut Job)) -> Option<Job> {
        let (owner, job) = {
            let mut g = self.lock();
            let e = g.iter_mut().find(|e| e.job.id == id)?;
            if e.job.is_finished() {
                return Some(e.job.clone());
            }
            f(&mut e.job);
            if e.job.is_finished() && e.finished_unix.is_none() {
                e.finished_unix = Some(unix_now());
                e.job.finished_at = Some(now_rfc3339());
            }
            e.refresh();
            e.last_emit = Some(Instant::now());
            self.persist_entry(e);
            (e.owner, e.job.clone())
        };
        self.emit(owner, job.clone());
        Some(job)
    }

    /// Updates items `positions` of a job with `f` (persisted; SSE at most every 200 ms per
    /// job, the next job update carries the rest).
    pub fn update_items(&self, id: &str, positions: &[usize], f: impl Fn(&mut JobItem)) {
        let emit = {
            let mut g = self.lock();
            let Some(e) = g.iter_mut().find(|e| e.job.id == id) else {
                return;
            };
            if e.job.is_finished() {
                return;
            }
            for &p in positions {
                if let Some(it) = e.job.items.get_mut(p) {
                    f(it);
                    if persisted(&e.job.kind) {
                        self.send(Persist::Item(id.to_string(), p, it.clone()));
                    }
                }
            }
            let due = e
                .last_emit
                .is_none_or(|t| t.elapsed() >= ITEM_EMIT_INTERVAL);
            if due {
                e.last_emit = Some(Instant::now());
                Some((e.owner, e.job.clone()))
            } else {
                None
            }
        };
        if let Some((owner, job)) = emit {
            self.emit(owner, job);
        }
    }

    /// The job's items (a snapshot).
    pub fn items(&self, id: &str) -> Vec<JobItem> {
        self.lock()
            .iter()
            .find(|e| e.job.id == id)
            .map(|e| e.job.items.clone())
            .unwrap_or_default()
    }

    pub fn running(&self, id: &str, progress: f64, message: &str) {
        self.update(id, |j| {
            j.state = "running".into();
            j.progress = progress.clamp(0.0, 1.0);
            j.message = message.into();
        });
    }

    pub fn log(&self, id: &str, line: &str) {
        self.update(id, |j| {
            j.log.push(line.into());
            if j.log.len() > MAX_LOG {
                let n = j.log.len() - MAX_LOG;
                j.log.drain(..n);
            }
        });
    }

    pub fn set_dir(&self, id: &str, dir: PathBuf) {
        let mut g = self.lock();
        if let Some(e) = g.iter_mut().find(|e| e.job.id == id) {
            e.dir = Some(dir);
            self.persist_entry(e);
        }
    }

    pub fn set_hint(&self, id: &str, hint: Option<JobHint>) {
        self.update(id, |j| j.hint = hint);
    }

    pub fn done(&self, id: &str, message: &str, file: Option<JobFile>) {
        if let Some(f) = file
            && let Some(e) = self.lock().iter_mut().find(|e| e.job.id == id)
        {
            e.job.download_url = Some(format!("/api/v1/jobs/{id}/download"));
            e.file = Some(f);
        }
        self.update(id, |j| {
            j.state = "done".into();
            j.progress = 1.0;
            j.message = message.into();
        });
    }

    pub fn fail(&self, id: &str, message: &str) {
        self.update(id, |j| {
            j.state = "failed".into();
            j.message = message.into();
        });
    }

    pub fn cancelled(&self, id: &str) {
        self.update(id, |j| {
            j.state = "cancelled".into();
            j.message = "Cancelled".into();
        });
    }

    fn visible(e: &Entry, user: &User) -> bool {
        e.owner == user.id || (user.is_admin() && e.job.kind == "import")
    }

    pub fn get(&self, id: &str, user: &User) -> Option<Job> {
        self.lock()
            .iter()
            .find(|e| e.job.id == id && Self::visible(e, user))
            .map(|e| e.job.clone())
    }

    /// Newest first, at most [`MAX_LIST`].
    pub fn list(&self, user: &User) -> Vec<Job> {
        self.lock()
            .iter()
            .rev()
            .filter(|e| Self::visible(e, user))
            .take(MAX_LIST)
            .map(|e| e.job.clone())
            .collect()
    }

    /// Requests cancellation; a queued job is cancelled at once, a running one when it notices.
    pub fn cancel(&self, id: &str, user: &User) -> Option<Job> {
        let queued = {
            let g = self.lock();
            let e = g
                .iter()
                .find(|e| e.job.id == id && Self::visible(e, user))?;
            e.cancel.store(true, Ordering::SeqCst);
            e.job.state == "queued"
        };
        if queued {
            self.cancelled(id);
        }
        self.get(id, user)
    }

    /// Puts a finished, retryable job of `user` back into the queue: books that already got
    /// through keep their result, the others are reset. Returns the job, its new
    /// cancellation flag and the stored request; `Err(None)` when there is no such job,
    /// `Err(Some(msg))` when it cannot be retried.
    pub fn requeue(
        &self,
        id: &str,
        user: &User,
        max_active: usize,
    ) -> Result<(Job, Arc<AtomicBool>, String), Option<String>> {
        let (owner, job, cancel, request) = {
            let mut g = self.lock();
            let active = g
                .iter()
                .filter(|e| e.owner == user.id && !e.job.is_finished())
                .count();
            let e = g
                .iter_mut()
                .find(|e| e.job.id == id && e.owner == user.id)
                .ok_or(None)?;
            if !e.job.retryable {
                return Err(Some("this job cannot be retried".into()));
            }
            if active >= max_active {
                return Err(Some(format!(
                    "at most {max_active} send/download jobs can run at once; wait for one to finish"
                )));
            }
            let request = e.request.clone().ok_or(None)?;
            e.cancel = Arc::new(AtomicBool::new(false));
            e.finished_unix = None;
            let j = &mut e.job;
            j.state = "queued".into();
            j.progress = 0.0;
            j.message = String::new();
            j.finished_at = None;
            j.hint = None;
            j.download_url = None;
            e.file = None;
            for it in &mut j.items {
                if !it.is_ok() {
                    it.state = "queued".into();
                    it.detail = String::new();
                }
            }
            e.refresh();
            e.last_emit = Some(Instant::now());
            self.persist_entry(e);
            self.send(Persist::Items(id.to_string(), e.job.items.clone()));
            (e.owner, e.job.clone(), e.cancel.clone(), request)
        };
        self.emit(owner, job.clone());
        Ok((job, cancel, request))
    }

    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.lock().len()
    }

    pub fn is_cancelled(&self, id: &str) -> bool {
        self.lock()
            .iter()
            .find(|e| e.job.id == id)
            .is_none_or(|e| e.cancel.load(Ordering::SeqCst))
    }

    /// The cancellation flag of a job (resumed jobs).
    pub fn cancel_flag(&self, id: &str) -> Option<Arc<AtomicBool>> {
        self.lock()
            .iter()
            .find(|e| e.job.id == id)
            .map(|e| e.cancel.clone())
    }

    /// Removes finished jobs visible to `user`; returns directories to delete.
    pub fn clear_finished(&self, user: &User) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let mut ids = Vec::new();
        self.lock().retain_mut(|e| {
            let remove = e.job.is_finished() && Self::visible(e, user);
            if remove {
                ids.push(e.job.id.clone());
                if let Some(d) = e.dir.take() {
                    dirs.push(d);
                }
            }
            !remove
        });
        self.send(Persist::Delete(ids));
        dirs
    }

    pub fn file(&self, id: &str, user: &User) -> Option<JobFile> {
        self.lock()
            .iter()
            .find(|e| e.job.id == id && Self::visible(e, user))
            .and_then(|e| e.file.clone())
    }

    /// Cancels and forgets every job of a deleted user; returns directories to delete.
    pub fn purge_user(&self, owner: i64) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        let mut ids = Vec::new();
        self.lock().retain_mut(|e| {
            if e.owner != owner {
                return true;
            }
            e.cancel.store(true, Ordering::SeqCst);
            ids.push(e.job.id.clone());
            if let Some(d) = e.dir.take() {
                dirs.push(d);
            }
            false
        });
        self.send(Persist::Delete(ids));
        dirs
    }

    /// Drops jobs finished more than `ttl_secs` ago (and trims the list); returns directories to delete.
    pub fn expire(&self, ttl_secs: i64) -> Vec<PathBuf> {
        let now = unix_now();
        let mut dirs = Vec::new();
        let mut ids = Vec::new();
        let mut g = self.lock();
        g.retain_mut(|e| {
            let old = e.finished_unix.is_some_and(|t| now - t > ttl_secs);
            if old {
                ids.push(e.job.id.clone());
                if let Some(d) = e.dir.take() {
                    dirs.push(d);
                }
            }
            !old
        });
        // hard cap on memory: drop the oldest *finished* jobs beyond 1000 entries (queued and
        // running jobs are bounded per user and must keep their state and cancel flag)
        if g.len() > 1000 {
            let mut excess = g.len() - 1000;
            g.retain_mut(|e| {
                if excess > 0 && e.job.is_finished() {
                    excess -= 1;
                    ids.push(e.job.id.clone());
                    if let Some(d) = e.dir.take() {
                        dirs.push(d);
                    }
                    false
                } else {
                    true
                }
            });
        }
        drop(g);
        if !ids.is_empty() {
            self.send(Persist::Delete(ids));
        }
        dirs
    }
}

fn write_ops(
    c: &rusqlite::Connection,
    ops: Vec<Persist>,
    acks: &mut Vec<oneshot::Sender<()>>,
) -> rusqlite::Result<()> {
    let tx = c.unchecked_transaction()?;
    for op in ops {
        match op {
            Persist::Job(r) => {
                let j = &r.job;
                tx.execute(
                    "INSERT INTO job(id, owner, kind, title, state, progress, message, log, request, hint, \
                     file_path, file_name, file_mime, dir, created_at, finished_at, finished_unix) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17) \
                     ON CONFLICT(id) DO UPDATE SET owner=excluded.owner, title=excluded.title, state=excluded.state, \
                     progress=excluded.progress, message=excluded.message, log=excluded.log, request=excluded.request, \
                     hint=excluded.hint, file_path=excluded.file_path, file_name=excluded.file_name, \
                     file_mime=excluded.file_mime, dir=excluded.dir, finished_at=excluded.finished_at, \
                     finished_unix=excluded.finished_unix",
                    rusqlite::params![
                        j.id,
                        r.owner,
                        j.kind,
                        j.title,
                        j.state,
                        j.progress,
                        j.message,
                        serde_json::to_string(&j.log).unwrap_or_else(|_| "[]".into()),
                        r.request,
                        j.hint.as_ref().and_then(|h| serde_json::to_string(h).ok()),
                        r.file.as_ref().map(|f| f.path.to_string_lossy().into_owned()),
                        r.file.as_ref().map(|f| f.name.clone()),
                        r.file.as_ref().map(|f| f.mime.clone()),
                        r.dir.as_ref().map(|d| d.to_string_lossy().into_owned()),
                        j.created_at,
                        j.finished_at,
                        r.finished_unix,
                    ],
                )?;
            }
            Persist::Item(id, pos, it) => write_item(&tx, &id, pos, &it)?,
            Persist::Items(id, items) => {
                tx.execute("DELETE FROM job_item WHERE job_id=?1", [&id])?;
                for (pos, it) in items.iter().enumerate() {
                    write_item(&tx, &id, pos, it)?;
                }
            }
            Persist::Delete(ids) => {
                for id in ids {
                    tx.execute("DELETE FROM job WHERE id=?1", [&id])?;
                }
            }
            Persist::Flush(a) => acks.push(a),
        }
    }
    tx.commit()
}

fn write_item(
    tx: &rusqlite::Connection,
    id: &str,
    pos: usize,
    it: &JobItem,
) -> rusqlite::Result<()> {
    // items of a job that is not (yet) in the table are skipped by the foreign key check
    tx.execute(
        "INSERT INTO job_item(job_id, pos, book_id, title, state, detail, attempts, size, mail, updated_at) \
         SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10 WHERE EXISTS (SELECT 1 FROM job WHERE id=?1) \
         ON CONFLICT(job_id, pos) DO UPDATE SET state=excluded.state, detail=excluded.detail, \
         attempts=excluded.attempts, size=excluded.size, mail=excluded.mail, updated_at=excluded.updated_at",
        rusqlite::params![
            id,
            pos as i64,
            it.book_id,
            it.title,
            it.state,
            it.detail,
            it.attempts,
            it.size.map(|s| s as i64),
            it.mail,
            now_rfc3339(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: i64) -> User {
        User {
            id,
            username: format!("u{id}"),
            role: "reader".into(),
        }
    }

    #[test]
    fn per_user_cap_and_trim() {
        let (tx, _) = broadcast::channel(16);
        let m = JobManager::new(tx);
        for _ in 0..3 {
            assert!(m.try_create("send", "x", 1, 3).is_some());
        }
        assert!(m.try_create("send", "x", 1, 3).is_none(), "cap reached");
        assert!(
            m.try_create("send", "x", 2, 3).is_some(),
            "other users unaffected"
        );
        let first = m.list(&user(1)).last().unwrap().id.clone();
        m.fail(&first, "boom");
        assert!(
            m.try_create("send", "x", 1, 3).is_some(),
            "finished jobs free a slot"
        );
        // the hard cap only drops finished jobs
        for i in 0..1100 {
            let (j, _) = m.create("send", "x", 100 + i);
            if i % 2 == 0 {
                m.done(&j.id, "ok", None);
            }
        }
        m.expire(i64::MAX);
        assert_eq!(m.count(), 1000);
        assert_eq!(
            m.list(&user(1)).iter().filter(|j| !j.is_finished()).count(),
            3,
            "active jobs survive trimming"
        );
        // purge
        let active = m.list(&user(2))[0].id.clone();
        m.purge_user(2);
        assert!(m.get(&active, &user(2)).is_none());
        assert!(m.is_cancelled(&active));
    }

    #[tokio::test]
    async fn persisted_resumed_and_retryable() {
        let dir = tempfile::tempdir().unwrap();
        let db = AppDb::open(&dir.path().join("app.db")).unwrap();
        db.lock()
            .execute(
                "INSERT INTO user(id, username, password_hash, role, created_at) VALUES (1,'u1','x','reader','t')",
                [],
            )
            .unwrap();
        let (tx, _) = broadcast::channel(64);
        let m = JobManager::new(tx.clone());
        m.attach_db(db.clone());
        let items = vec![JobItem::new(1, "A"), JobItem::new(2, "B")];
        let (running, _) = m
            .try_create_with("send", "run", 1, 5, Some("{\"r\":1}".into()), items.clone())
            .unwrap();
        let (queued, _) = m
            .try_create_with(
                "send",
                "queue",
                1,
                5,
                Some("{\"r\":2}".into()),
                items.clone(),
            )
            .unwrap();
        let (done, _) = m
            .try_create_with("export", "done", 1, 5, Some("{}".into()), items.clone())
            .unwrap();
        let (import, _) = m.create("import", "imp", 1);
        m.running(&running.id, 0.5, "sending");
        m.update_items(&running.id, &[0], |it| {
            it.state = "accepted".into();
            it.detail = "250 queued as X".into();
        });
        m.update_items(&running.id, &[1], |it| it.state = "sending".into());
        m.update_items(&done.id, &[0, 1], |it| it.state = "saved".into());
        m.done(&done.id, "2 done", None);
        m.flush().await;

        // "restart"
        let m2 = JobManager::new(tx);
        let resume = m2.load(&db).unwrap();
        m2.attach_db(db.clone());
        assert_eq!(
            resume,
            vec![(queued.id.clone(), 1, "{\"r\":2}".to_string())]
        );
        let u = user(1);
        let r = m2.get(&running.id, &u).unwrap();
        assert_eq!(r.state, "failed");
        assert!(r.retryable && r.message.contains("restart"));
        assert_eq!(r.items[0].state, "accepted");
        assert_eq!(r.items[0].detail, "250 queued as X");
        assert_eq!(r.items[1].state, "failed");
        let d = m2.get(&done.id, &u).unwrap();
        assert_eq!((d.state.as_str(), d.retryable), ("done", false));
        assert!(
            m2.get(&import.id, &u).is_none(),
            "imports are not persisted"
        );
        // retry: the accepted book keeps its result
        let (j, _, req) = m2.requeue(&running.id, &u, 5).unwrap();
        assert_eq!(req, "{\"r\":1}");
        assert_eq!(j.state, "queued");
        assert_eq!(j.items[0].state, "accepted");
        assert_eq!(j.items[1].state, "queued");
        assert!(m2.requeue(&done.id, &u, 5).is_err(), "nothing to retry");
        assert!(matches!(m2.requeue("nope", &u, 5), Err(None)));
        m2.flush().await;
        // clearing finished jobs removes them from the database too
        m2.clear_finished(&u);
        m2.flush().await;
        let n: i64 = db
            .lock()
            .query_row("SELECT count(*) FROM job", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2, "the queued and the requeued job remain");
    }
}
