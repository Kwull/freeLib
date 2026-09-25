//! Library imports as jobs: freelib-import in a blocking thread, progress → job events,
//! catalog swap-in and cache warm-up.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use freelib_import::{ImportError, ImportOptions};

use crate::db::{self, User};
use crate::error::{ApiError, ApiResult};
use crate::jobs::Job;
use crate::state::{AppState, ImportRun, LibRuntime, LibraryStatus};

/// Starts an import of library `lib_id`; 409 when one is already running.
pub async fn start(st: &AppState, lib_id: i64, owner: &User) -> ApiResult<Job> {
    start_with_reason(st, lib_id, owner, None).await
}

/// [`start`] with the `status.reason` of an import the server started by itself.
pub async fn start_with_reason(
    st: &AppState,
    lib_id: i64,
    owner: &User,
    reason: Option<&'static str>,
) -> ApiResult<Job> {
    let rt = st.lib(lib_id)?;
    let row = st
        .db
        .run(move |c| db::get_library(c, lib_id))
        .await?
        .ok_or_else(|| ApiError::not_found("library not found"))?;
    let Some(inpx) = row.inpx.clone() else {
        return Err(ApiError::bad_request("library has no INPX file"));
    };
    let (job, cancel) = {
        let mut g = rt.import.lock().unwrap_or_else(|e| e.into_inner());
        if rt.is_deleted() {
            return Err(ApiError::not_found("library not found"));
        }
        if g.is_some() {
            return Err(ApiError::conflict(
                "an import of this library is already running",
            ));
        }
        let (job, cancel) = st
            .jobs
            .create("import", &format!("Import · {}", row.name), owner.id);
        *g = Some(ImportRun {
            job_id: job.id.clone(),
        });
        (job, cancel)
    };
    rt.set_status(LibraryStatus {
        state: "importing".into(),
        progress: Some(0.0),
        message: None,
        reason: reason.map(String::from),
    });
    st.emit_library(lib_id);
    let opts = ImportOptions {
        inpx: PathBuf::from(inpx),
        db_path: rt.handle.path().to_path_buf(),
        library_dir: Some(PathBuf::from(&row.path)),
        first_author_only: row.first_author_only,
        skip_deleted: row.skip_deleted,
        resolve_offsets: true,
        threads: 0,
    };
    let st2 = st.clone();
    let job_id = job.id.clone();
    tokio::spawn(async move { run(st2, rt, job_id, opts, cancel, reason).await });
    Ok(job)
}

async fn run(
    st: AppState,
    rt: Arc<LibRuntime>,
    job_id: String,
    opts: ImportOptions,
    cancel: Arc<AtomicBool>,
    reason: Option<&'static str>,
) {
    st.jobs.running(&job_id, 0.0, "Starting");
    st.jobs
        .log(&job_id, &format!("Reading {}", opts.inpx.display()));
    let st2 = st.clone();
    let rt2 = rt.clone();
    let jid = job_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        let last_emit = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));
        let progress = |done: u64, total: u64, msg: &str| {
            let p = if total > 0 {
                done as f64 / total as f64
            } else {
                0.0
            };
            st2.jobs.running(&jid, p, msg);
            let step = (total / 10).max(1);
            if !msg.is_empty()
                && (done == 0
                    || done + freelib_import::builder::FINISH_STEPS >= total
                    || done % step == 0)
            {
                st2.jobs.log(&jid, msg);
            }
            rt2.set_status(LibraryStatus {
                state: "importing".into(),
                progress: Some(p),
                message: Some(msg.into()),
                reason: reason.map(String::from),
            });
            if done >= total && !rt2.is_deleted() {
                // the new file was just renamed into place: switch to it at once, so requests
                // never open connections to the new file through the old catalog (stale)
                if rt2.handle.reload().is_ok() {
                    rt2.clear_lists();
                }
            }
            let mut le = last_emit.lock().unwrap_or_else(|e| e.into_inner());
            if le.elapsed() > Duration::from_millis(500) || done >= total {
                *le = Instant::now();
                st2.emit_library(rt2.id);
            }
        };
        freelib_import::import_inpx(&opts, &progress, &cancel)
    })
    .await;
    finish(&st, &rt, &job_id, result);
    // the slot is released last: a delete waits for (is refused during) all of the above
    *rt.import.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn finish(
    st: &AppState,
    rt: &Arc<LibRuntime>,
    job_id: &str,
    result: Result<Result<freelib_import::ImportStats, ImportError>, tokio::task::JoinError>,
) {
    let job_id = job_id.to_string();
    if rt.is_deleted() {
        // deleted while importing: throw the result away
        rt.handle.close();
        let _ = std::fs::remove_file(rt.handle.path());
        let _ = std::fs::remove_file(freelib_import::new_db_path(rt.handle.path()));
        st.jobs.cancelled(&job_id);
        return;
    }
    match result {
        Ok(Ok(stats)) => {
            let reload = rt.handle.reload();
            rt.clear_lists();
            match reload {
                Ok(cat) => {
                    let mut msg = format!(
                        "{} books, {} authors, {} series in {:.1} s",
                        stats.live_books,
                        stats.authors,
                        stats.series,
                        stats.elapsed_ms as f64 / 1000.0
                    );
                    if !stats.missing_archives.is_empty() {
                        msg.push_str(&format!(
                            "; {} archives missing",
                            stats.missing_archives.len()
                        ));
                        for a in stats.missing_archives.iter().take(20) {
                            st.jobs.log(&job_id, &format!("Missing archive: {a}"));
                        }
                    }
                    st.jobs.log(&job_id, &msg);
                    rt.set_status(LibraryStatus::default());
                    st.jobs.done(&job_id, &msg, None);
                    st.emit_library(rt.id);
                    warm(st, rt, cat);
                }
                Err(e) => {
                    let m = format!("imported catalog cannot be opened: {e}");
                    rt.set_status(LibraryStatus {
                        state: "error".into(),
                        progress: None,
                        message: Some(m.clone()),
                        reason: None,
                    });
                    st.jobs.fail(&job_id, &m);
                    st.emit_library(rt.id);
                }
            }
        }
        Ok(Err(ImportError::Cancelled)) => {
            rt.set_status(LibraryStatus::default());
            st.jobs.cancelled(&job_id);
            st.emit_library(rt.id);
        }
        Ok(Err(e)) => {
            let m = format!("import failed: {e}");
            tracing::warn!(lib = rt.id, "{m}");
            rt.set_status(LibraryStatus {
                state: "error".into(),
                progress: None,
                message: Some(m.clone()),
                reason: None,
            });
            st.jobs.log(&job_id, &m);
            st.jobs.fail(&job_id, &m);
            st.emit_library(rt.id);
        }
        Err(e) => {
            let m = format!("import task failed: {e}");
            rt.set_status(LibraryStatus {
                state: "error".into(),
                progress: None,
                message: Some(m.clone()),
                reason: None,
            });
            st.jobs.fail(&job_id, &m);
            st.emit_library(rt.id);
        }
    }
}

/// Loads the search attributes and pre-builds the authors/series lists in the background.
pub fn warm(st: &AppState, rt: &Arc<LibRuntime>, cat: Arc<freelib_catalog::Catalog>) {
    let st = st.clone();
    let rt = rt.clone();
    tokio::task::spawn_blocking(move || {
        let t = Instant::now();
        for kind in ["authors", "series"] {
            if let Ok(body) = crate::api::browse::build_list(&rt, &cat, kind) {
                body.encoded("br");
            }
        }
        if let Err(e) = cat.attrs() {
            tracing::warn!(lib = rt.id, "attribute load failed: {e}");
        }
        for kind in ["authors", "series"] {
            if let Ok(body) = crate::api::browse::build_list(&rt, &cat, kind) {
                body.encoded("gzip");
            }
        }
        let _ = &st;
        tracing::info!(
            lib = rt.id,
            ms = t.elapsed().as_millis() as u64,
            "catalog warmed up"
        );
    });
}
