//! In-memory job list (imports, send/export/download) with SSE events.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::broadcast;

use crate::db::User;
use crate::util::{now_rfc3339, random_id, unix_now};

pub const MAX_LOG: usize = 50;
/// Jobs kept per user in `GET /jobs`.
pub const MAX_LIST: usize = 50;

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
}

/// Server-sent events.
#[derive(Debug, Clone)]
pub enum Event {
    Job { owner: i64, job: Job },
    Library { id: i64 },
}

impl Event {
    /// Whether `user` may see this event.
    pub fn visible_to(&self, user: &User) -> bool {
        match self {
            Event::Job { owner, job } => *owner == user.id || (user.is_admin() && job.kind == "import"),
            Event::Library { .. } => true,
        }
    }
}

pub struct JobManager {
    entries: Mutex<Vec<Entry>>,
    pub events: broadcast::Sender<Event>,
}

impl JobManager {
    pub fn new(events: broadcast::Sender<Event>) -> JobManager {
        JobManager { entries: Mutex::new(Vec::new()), events }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, owner: i64, job: Job) {
        let _ = self.events.send(Event::Job { owner, job });
    }

    /// Creates a queued job; returns it and its cancellation flag.
    pub fn create(&self, kind: &str, title: &str, owner: i64) -> (Job, Arc<AtomicBool>) {
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
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.lock().push(Entry {
            job: job.clone(),
            owner,
            cancel: cancel.clone(),
            file: None,
            finished_unix: None,
            dir: None,
        });
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
            (e.owner, e.job.clone())
        };
        self.emit(owner, job.clone());
        Some(job)
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
        if let Some(e) = self.lock().iter_mut().find(|e| e.job.id == id) {
            e.dir = Some(dir);
        }
    }

    pub fn done(&self, id: &str, message: &str, file: Option<JobFile>) {
        if let Some(f) = file {
            if let Some(e) = self.lock().iter_mut().find(|e| e.job.id == id) {
                e.job.download_url = Some(format!("/api/v1/jobs/{id}/download"));
                e.file = Some(f);
            }
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
        self.lock().iter().find(|e| e.job.id == id && Self::visible(e, user)).map(|e| e.job.clone())
    }

    /// Newest first, at most [`MAX_LIST`].
    pub fn list(&self, user: &User) -> Vec<Job> {
        self.lock().iter().rev().filter(|e| Self::visible(e, user)).take(MAX_LIST).map(|e| e.job.clone()).collect()
    }

    /// Requests cancellation; a queued job is cancelled at once, a running one when it notices.
    pub fn cancel(&self, id: &str, user: &User) -> Option<Job> {
        let queued = {
            let g = self.lock();
            let e = g.iter().find(|e| e.job.id == id && Self::visible(e, user))?;
            e.cancel.store(true, Ordering::SeqCst);
            e.job.state == "queued"
        };
        if queued {
            self.cancelled(id);
        }
        self.get(id, user)
    }

    pub fn is_cancelled(&self, id: &str) -> bool {
        self.lock().iter().find(|e| e.job.id == id).is_none_or(|e| e.cancel.load(Ordering::SeqCst))
    }

    /// Removes finished jobs visible to `user`; returns directories to delete.
    pub fn clear_finished(&self, user: &User) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        self.lock().retain_mut(|e| {
            let remove = e.job.is_finished() && Self::visible(e, user);
            if remove && let Some(d) = e.dir.take() {
                dirs.push(d);
            }
            !remove
        });
        dirs
    }

    pub fn file(&self, id: &str, user: &User) -> Option<JobFile> {
        self.lock().iter().find(|e| e.job.id == id && Self::visible(e, user)).and_then(|e| e.file.clone())
    }

    /// Drops jobs finished more than `ttl_secs` ago (and trims the list); returns directories to delete.
    pub fn expire(&self, ttl_secs: i64) -> Vec<PathBuf> {
        let now = unix_now();
        let mut dirs = Vec::new();
        let mut g = self.lock();
        g.retain_mut(|e| {
            let old = e.finished_unix.is_some_and(|t| now - t > ttl_secs);
            if old && let Some(d) = e.dir.take() {
                dirs.push(d);
            }
            !old
        });
        // hard cap on memory: keep the newest 1000 entries
        if g.len() > 1000 {
            let n = g.len() - 1000;
            for mut e in g.drain(..n) {
                if let Some(d) = e.dir.take() {
                    dirs.push(d);
                }
            }
        }
        dirs
    }
}
