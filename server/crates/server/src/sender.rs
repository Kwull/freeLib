//! `POST /send` jobs: e-mail (Send to Kindle), export to a server folder, download (file or zip).
//!
//! E-mail jobs first convert every book, then pack the files into as few mails as the limits
//! allow (`smtp.maxAttachments`, `smtp.maxMailMb`, see [`plan_mails`]) and deliver them with
//! automatic retries of temporary failures (`smtp.retries`, `smtp.retryDelaySeconds`, ×4 per
//! retry; see [`crate::mail`] for the classification). Every book has its own delivery state
//! in the job ([`JobItem`]); jobs survive restarts ([`crate::jobs`]).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use freelib_catalog::BookDetail;
use freelib_fb2conv::ConvertOptions;
use serde::{Deserialize, Serialize};

use crate::db::{self, Device, SmtpConfig, User};
use crate::error::{ApiError, ApiResult};
use crate::jobs::{Job, JobFile, JobHint, JobItem, MAX_ITEMS, Resume};
use crate::output::{self, Produced};
use crate::state::AppState;
use crate::util::{join_safe, mime_for_ext, safe_subdir};

pub const MAX_BOOKS: usize = 1000;
/// Series per request ("send whole series").
pub const MAX_SERIES: usize = 20;

/// Amazon's page where the sender address has to be approved.
pub const KINDLE_APPROVED_URL: &str = "https://www.amazon.com/hz/mycd/myx#/home/settings/payment";

pub struct SendRequest {
    pub library: i64,
    pub books: Vec<i64>,
    /// Whole series (catalog ids): their live books in reading order, duplicates removed.
    pub series: Vec<i64>,
    pub device: Device,
    pub target: Option<String>,
    pub file_name: Option<String>,
}

/// What a job needs to run again (resume after a restart, retry); stored with the job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRequest {
    pub library: i64,
    pub books: Vec<i64>,
    pub device: Device,
    pub target: Option<String>,
    pub file_name: Option<String>,
}

/// One output file: a book, or several books of one series joined.
struct Unit {
    books: Vec<BookDetail>,
}

/// Groups files into mails: at most `max_count` attachments and `max_bytes` (the encoded size,
/// see [`encoded_size`]) per mail, in the given order. Returns the mails (indices into `sizes`)
/// and the files that do not fit into any mail.
pub fn plan_mails(
    sizes: &[u64],
    max_count: usize,
    max_bytes: u64,
) -> (Vec<Vec<usize>>, Vec<usize>) {
    let max_count = max_count.max(1);
    let mut mails: Vec<Vec<usize>> = Vec::new();
    let mut too_big = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_bytes = 0u64;
    for (i, &s) in sizes.iter().enumerate() {
        if s > max_bytes {
            too_big.push(i);
            continue;
        }
        if !cur.is_empty() && (cur.len() >= max_count || cur_bytes + s > max_bytes) {
            mails.push(std::mem::take(&mut cur));
            cur_bytes = 0;
        }
        cur.push(i);
        cur_bytes += s;
    }
    if !cur.is_empty() {
        mails.push(cur);
    }
    (mails, too_big)
}

/// Size of a file as a base64 MIME attachment (what mail size limits measure), with a little
/// room for headers.
pub fn encoded_size(raw: u64) -> u64 {
    raw.div_ceil(3) * 4 + raw / 57 * 2 + 1024
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// Whether mails to `to` go to Amazon's Send to Kindle service.
pub fn is_kindle_address(to: &str) -> bool {
    let d = to.trim().to_ascii_lowercase();
    d.ends_with("@kindle.com") || d.ends_with("@free.kindle.com")
}

/// Live books of `series` (reading order), each title once: of several editions with the same
/// title and language, the newest (by catalog date) is kept.
async fn series_books(st: &AppState, lib: i64, series: Vec<i64>) -> ApiResult<Vec<i64>> {
    st.catalog_call(lib, move |cat| {
        let mut out: Vec<(i64, String)> = Vec::new();
        for sid in &series {
            if cat.series(*sid)?.is_none() {
                continue;
            }
            let page = cat.books(
                &freelib_catalog::BookSelector::Series(*sid),
                &freelib_catalog::BookFilter::default(),
                &freelib_catalog::Page {
                    cursor: None,
                    limit: MAX_BOOKS,
                },
            )?;
            let mut seen: HashMap<String, usize> = HashMap::new();
            for b in page.books {
                let key = format!(
                    "{}\0{}",
                    b.title
                        .chars()
                        .filter(|c| c.is_alphanumeric())
                        .flat_map(char::to_lowercase)
                        .collect::<String>(),
                    b.lang
                );
                match seen.get(&key) {
                    Some(&i) => {
                        if b.date > out[i].1 {
                            out[i] = (b.id, b.date.clone());
                        }
                    }
                    None => {
                        seen.insert(key, out.len());
                        out.push((b.id, b.date.clone()));
                    }
                }
            }
        }
        Ok(out.into_iter().map(|(id, _)| id).collect())
    })
    .await
}

pub async fn start(st: &AppState, user: &User, mut req: SendRequest) -> ApiResult<Job> {
    if req.series.len() > MAX_SERIES {
        return Err(ApiError::bad_request(format!(
            "at most {MAX_SERIES} series per request"
        )));
    }
    let mut series_name = None;
    if !req.series.is_empty() {
        st.catalog(req.library)?;
        let sid = req.series[0];
        series_name = st
            .catalog_call(req.library, move |cat| Ok(cat.series(sid)?.map(|s| s.name)))
            .await?;
        let mut ids = series_books(st, req.library, req.series.clone()).await?;
        if ids.is_empty() {
            return Err(ApiError::not_found("no books in this series"));
        }
        // explicitly selected books first, then the series (each book once)
        let mut all = std::mem::take(&mut req.books);
        ids.retain(|id| !all.contains(id));
        all.extend(ids);
        req.books = all;
    }
    if req.books.is_empty() {
        return Err(ApiError::bad_request("no books selected"));
    }
    if req.books.len() > MAX_BOOKS {
        return Err(ApiError::bad_request(format!(
            "at most {MAX_BOOKS} books per request"
        )));
    }
    let dev = &req.device;
    if matches!(dev.format.as_str(), "azw3" | "mobi" | "pdf") && st.calibre.is_none() {
        return Err(ApiError::unsupported(format!(
            "{} needs Calibre, which is not installed",
            dev.format
        )));
    }
    let target = req
        .target
        .clone()
        .filter(|t| !t.trim().is_empty())
        .or(dev.target.clone());
    match dev.kind.as_str() {
        "email" => {
            let to = target.clone().unwrap_or_default();
            if !to.contains('@') {
                return Err(ApiError::bad_request("an e-mail address is required"));
            }
            let smtp: SmtpConfig = st
                .db
                .run(|c| db::get_setting::<SmtpConfig>(c, "smtp"))
                .await?
                .revealed(&st.secrets)?;
            if smtp.host.trim().is_empty() {
                return Err(ApiError::bad_request(
                    "SMTP server is not configured (Settings → Mail)",
                ));
            }
            crate::api::devices::check_recipient(&smtp, &to)?;
            let (uid, day) = (user.id, local_today());
            let used = st.db.run(move |c| db::mail_count(c, uid, &day)).await?;
            let limit = i64::from(smtp.daily_limit_per_user);
            // books travel together, so a request needs at least this many mails
            let mails = req
                .books
                .len()
                .div_ceil(smtp.max_attachments.max(1) as usize) as i64;
            if used + mails > limit {
                return Err(ApiError::rate_limited(format!(
                    "daily mail limit reached: {used} of {limit} mails sent today"
                )));
            }
        }
        "folder" => {
            // server folders are admin business: readers may use shared folder devices as
            // configured, but neither own folder devices nor another target folder
            if !user.is_admin()
                && (!dev.shared
                    || req.target.as_deref().is_some_and(|t| {
                        !t.trim().is_empty() && Some(t.trim()) != dev.target.as_deref()
                    }))
            {
                return Err(ApiError::forbidden(
                    "only administrators can choose server folders",
                ));
            }
            if safe_subdir(target.as_deref().unwrap_or("")).is_none() {
                return Err(ApiError::bad_request("invalid target folder"));
            }
        }
        "download" => {}
        _ => return Err(ApiError::bad_request("unknown device kind")),
    }
    st.catalog(req.library)?;
    // per-book items (titles from the catalog, in request order)
    let items = if req.books.len() <= MAX_ITEMS {
        let ids = req.books.clone();
        let books = st
            .catalog_call(req.library, move |cat| Ok(cat.books_by_ids(&ids)?))
            .await?;
        let by_id: HashMap<i64, String> = books.into_iter().map(|b| (b.id, b.title)).collect();
        req.books.retain(|id| by_id.contains_key(id));
        if req.books.is_empty() {
            return Err(ApiError::not_found("books not found"));
        }
        req.books
            .iter()
            .map(|id| JobItem::new(*id, &by_id[id]))
            .collect()
    } else {
        Vec::new()
    };
    let n = req.books.len();
    let what = match &series_name {
        Some(s) => format!("«{s}» · {}", plural(n, "book")),
        None => plural(n, "book"),
    };
    let (kind, title) = match dev.kind.as_str() {
        "email" => ("send", format!("Send to {} · {what}", dev.name)),
        "folder" => ("export", format!("Export to {} · {what}", dev.name)),
        _ => ("download", format!("Download for {} · {what}", dev.name)),
    };
    let stored = StoredRequest {
        library: req.library,
        books: req.books.clone(),
        device: req.device.clone(),
        target,
        file_name: req.file_name.clone(),
    };
    let request = serde_json::to_string(&stored).map_err(|e| ApiError::internal(e.to_string()))?;
    let max = st.cfg.max_jobs_per_user;
    let (job, cancel) = st
        .jobs
        .try_create_with(kind, &title, user.id, max, Some(request), items)
        .ok_or_else(|| {
            ApiError::rate_limited(format!(
                "at most {max} send/download jobs can run at once; wait for one to finish"
            ))
        })?;
    let k = req.books.len().min(crate::extrating::BROWSE_ENQUEUE);
    st.ext.enqueue(
        crate::extrating::Priority::User,
        req.library,
        &req.books[..k],
    );
    spawn_run(st, job.id.clone(), user.id, stored, cancel);
    Ok(job)
}

/// Runs a (new, resumed or retried) job in the background.
pub fn spawn_run(
    st: &AppState,
    job_id: String,
    uid: i64,
    req: StoredRequest,
    cancel: Arc<AtomicBool>,
) {
    let st2 = st.clone();
    tokio::spawn(async move {
        let (lib, dev_name) = (req.library, req.device.name.clone());
        let action = if req.device.kind == "download" {
            "download"
        } else {
            "send"
        };
        match run(&st2, &job_id, uid, req, cancel).await {
            Ok(ok_ids) => record_history(&st2, uid, lib, ok_ids, action, dev_name).await,
            Err(e) if e.code == "cancelled" => st2.jobs.cancelled(&job_id),
            Err(e) => st2.jobs.fail(&job_id, &e.message),
        }
    });
}

/// Resumes the jobs that were queued when the server stopped.
pub fn resume(st: &AppState, jobs: Vec<Resume>) {
    for (id, owner, request) in jobs {
        let Some(cancel) = st.jobs.cancel_flag(&id) else {
            continue;
        };
        match serde_json::from_str::<StoredRequest>(&request) {
            Ok(r) => {
                tracing::info!("resuming job {id}");
                spawn_run(st, id, owner, r, cancel);
            }
            Err(e) => st.jobs.fail(&id, &format!("cannot resume: {e}")),
        }
    }
}

/// `POST /jobs/:id/retry`: runs a failed (or partly failed) job again; books that already got
/// through are not sent twice.
pub fn retry(st: &AppState, user: &User, id: &str) -> ApiResult<Job> {
    let (job, cancel, request) = st
        .jobs
        .requeue(id, user, st.cfg.max_jobs_per_user)
        .map_err(|e| match e {
            None => ApiError::not_found("job not found"),
            Some(m) => ApiError::conflict(m),
        })?;
    let r: StoredRequest =
        serde_json::from_str(&request).map_err(|e| ApiError::internal(e.to_string()))?;
    spawn_run(st, job.id.clone(), user.id, r, cancel);
    Ok(job)
}

/// Remembers what the user sent / downloaded (reading profile, "already sent").
async fn record_history(
    st: &AppState,
    uid: i64,
    lib: i64,
    ids: Vec<i64>,
    action: &'static str,
    device: String,
) {
    if ids.is_empty() {
        return;
    }
    let Ok(keys) = st
        .catalog_call(lib, move |cat| Ok(cat.keys_by_ids(&ids)?))
        .await
    else {
        return;
    };
    let keys: Vec<String> = keys.into_iter().map(|(_, k)| k).collect();
    let _ = st
        .db
        .run(move |c| db::add_history(c, uid, lib, &keys, action, Some(&device)))
        .await;
}

fn cancelled() -> ApiError {
    ApiError::new(axum::http::StatusCode::OK, "cancelled", "cancelled")
}

/// Today in the server's local time zone (`YYYY-MM-DD`), the day of the mail limit.
fn local_today() -> String {
    crate::util::local_date_at(crate::util::unix_now())
}

/// Creates `dir/rel` without replacing an existing file: `name (2).ext`, `name (3).ext`, ….
async fn create_unique(dir: &Path, rel: &str) -> ApiResult<(PathBuf, tokio::fs::File)> {
    let path = join_safe(dir, rel).ok_or_else(|| ApiError::bad_request("invalid file name"))?;
    if let Some(p) = path.parent() {
        tokio::fs::create_dir_all(p).await?;
    }
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // `kepub.epub` stays one extension
    let (stem, ext) = match file_name.find(".kepub.epub") {
        Some(i) => (file_name[..i].to_string(), file_name[i..].to_string()),
        None => match file_name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (file_name.clone(), String::new()),
        },
    };
    for k in 1..1000 {
        let candidate = if k == 1 {
            path.clone()
        } else {
            path.with_file_name(format!("{stem} ({k}){ext}"))
        };
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
            .await
        {
            Ok(f) => return Ok((candidate, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(ApiError::conflict("too many files with this name"))
}

/// Sleeps `d`, waking early (with `Err`) when the job is cancelled.
async fn sleep_cancellable(d: Duration, cancel: &AtomicBool) -> ApiResult<()> {
    let end = tokio::time::Instant::now() + d;
    while tokio::time::Instant::now() < end {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        let left = end - tokio::time::Instant::now();
        tokio::time::sleep(left.min(Duration::from_millis(250))).await;
    }
    Ok(())
}

/// A converted unit waiting to be mailed.
struct Ready {
    unit: usize,
    path: PathBuf,
    name: String,
    mime: String,
    size: u64,
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// Runs the job; returns the ids of the books that got through.
async fn run(
    st: &AppState,
    job_id: &str,
    user_id: i64,
    req: StoredRequest,
    cancel: Arc<AtomicBool>,
) -> ApiResult<Vec<i64>> {
    let dev = req.device.clone();
    let ids = req.books.clone();
    let details: Vec<BookDetail> = st
        .catalog_call(req.library, move |cat| {
            let mut v = Vec::with_capacity(ids.len());
            for id in &ids {
                if let Some(b) = cat.book(*id)? {
                    v.push(b);
                }
            }
            Ok(v)
        })
        .await?;
    if details.is_empty() {
        return Err(ApiError::not_found("books not found"));
    }
    let lib_dir = crate::api::books::lib_dir(st, req.library).await?;
    let template = req
        .file_name
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or(dev.file_name.clone());
    let opts = dev.options.clone();
    let units = make_units(details, &dev.format, &opts);
    // item positions of each unit (none for jobs without per-book items)
    let items = st.jobs.items(job_id);
    let mut pos_of: HashMap<i64, usize> = HashMap::new();
    for (p, it) in items.iter().enumerate() {
        pos_of.insert(it.book_id, p);
    }
    let unit_pos: Vec<Vec<usize>> = units
        .iter()
        .map(|u| {
            u.books
                .iter()
                .filter_map(|b| pos_of.get(&b.book.id).copied())
                .collect()
        })
        .collect();
    // books that already got through on an earlier run (retry)
    let already = |i: usize| -> bool {
        dev.kind != "download"
            && !unit_pos[i].is_empty()
            && unit_pos[i].iter().all(|&p| items[p].is_ok())
    };
    let total = units.len();
    let multi_file = total > 1;
    let job_dir = st.cfg.cache_dir.join("jobs").join(job_id);
    let mut produced: Vec<(String, PathBuf)> = Vec::new();
    let mut ready: Vec<Ready> = Vec::new();
    let mut ok_units = vec![false; total];
    let mut failed_units = vec![false; total];
    let set = |i: usize, f: &dyn Fn(&mut JobItem)| st.jobs.update_items(job_id, &unit_pos[i], f);

    // phase 1: convert (and, for folders and downloads, deliver)
    for (i, unit) in units.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) || st.jobs.is_cancelled(job_id) {
            return Err(cancelled());
        }
        if already(i) {
            ok_units[i] = true;
            continue;
        }
        let first = &unit.books[0];
        let phase = if dev.kind == "email" { 0.6 } else { 1.0 };
        st.jobs.running(
            job_id,
            i as f64 / total as f64 * phase,
            &format!("Converting {}", first.book.title),
        );
        set(i, &|it| {
            it.state = "converting".into();
            it.detail = String::new();
        });
        let res = produce_unit(st, req.library, &lib_dir, unit, &dev.format, &opts, &cancel).await;
        let (data, ext) = match res {
            Ok(x) => x,
            Err(e) if e.code == "cancelled" => return Err(cancelled()),
            Err(e) => {
                failed_units[i] = true;
                let msg = e.message.clone();
                set(i, &|it| {
                    it.state = "failed".into();
                    it.detail = msg.clone();
                });
                st.jobs
                    .log(job_id, &format!("{}: {}", first.book.title, e.message));
                continue;
            }
        };
        let mut book = first.book.clone();
        if unit.books.len() > 1 {
            // joined series: named after the series
            book.title = book
                .series
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or(book.title);
            book.serno = None;
        }
        let base = st
            .conv
            .file_name(&template, &output::name_fields(&book), opts.transliterate);
        match dev.kind.as_str() {
            "email" => {
                tokio::fs::create_dir_all(&job_dir).await?;
                st.jobs.set_dir(job_id, job_dir.clone());
                let path = job_dir.join(format!("mail-{i:05}.part"));
                data.write_to(&path).await?;
                let size = tokio::fs::metadata(&path).await?.len();
                set(i, &|it| {
                    it.state = "converted".into();
                    it.size = Some(size);
                });
                ready.push(Ready {
                    unit: i,
                    path,
                    name: format!("{}.{ext}", base.replace('/', " - ")),
                    mime: mime_for_ext(ext.rsplit('.').next().unwrap_or(&ext)).to_string(),
                    size,
                });
            }
            "folder" => {
                let sub = safe_subdir(req.target.as_deref().unwrap_or("")).unwrap_or_default();
                let dir = st.cfg.export_dir.join(sub);
                // never overwrite: an existing file gets a " (2)" sibling
                let (path, f) = create_unique(&dir, &format!("{base}.{ext}")).await?;
                drop(f);
                if let Err(e) = data.write_to(&path).await {
                    let _ = tokio::fs::remove_file(&path).await;
                    return Err(e);
                }
                let shown = path
                    .strip_prefix(&dir)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| format!("{base}.{ext}"));
                let size = tokio::fs::metadata(&path).await.map(|m| m.len()).ok();
                set(i, &|it| {
                    it.state = "saved".into();
                    it.detail = shown.clone();
                    it.size = size;
                });
                ok_units[i] = true;
                st.jobs.log(job_id, &format!("Saved {shown}"));
            }
            _ => {
                let rel = if multi_file {
                    format!("{base}.{ext}")
                } else {
                    format!("{}.{ext}", base.replace('/', " - "))
                };
                tokio::fs::create_dir_all(&job_dir).await?;
                st.jobs.set_dir(job_id, job_dir.clone());
                let p = job_dir.join(format!("{i:05}.part"));
                data.write_to(&p).await?;
                let size = tokio::fs::metadata(&p).await.map(|m| m.len()).ok();
                set(i, &|it| {
                    it.state = "ready".into();
                    it.size = size;
                });
                ok_units[i] = true;
                produced.push((rel, p));
            }
        }
    }

    // phase 2 (e-mail): pack into mails and deliver
    let mut mails_sent = 0usize;
    let mut hint = None;
    if dev.kind == "email" && !ready.is_empty() {
        let smtp: SmtpConfig = st
            .db
            .run(|c| db::get_setting::<SmtpConfig>(c, "smtp"))
            .await?
            .revealed(&st.secrets)?;
        let to = req.target.clone().unwrap_or_default();
        let max_bytes = u64::from(smtp.max_mail_mb.max(1)) * 1024 * 1024;
        let sizes: Vec<u64> = ready.iter().map(|r| encoded_size(r.size)).collect();
        let (mails, too_big) = plan_mails(&sizes, smtp.max_attachments as usize, max_bytes);
        for &k in &too_big {
            let r = &ready[k];
            failed_units[r.unit] = true;
            let msg = format!(
                "{} is too large for one e-mail (limit {} MB)",
                mb(r.size),
                smtp.max_mail_mb
            );
            set(r.unit, &|it| {
                it.state = "failed".into();
                it.detail = msg.clone();
            });
            st.jobs.log(job_id, &format!("{}: {msg}", r.name));
        }
        let n_mails = mails.len();
        for (m, mail) in mails.iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                return Err(cancelled());
            }
            if m > 0 && smtp.pause_seconds > 0 {
                sleep_cancellable(Duration::from_secs(smtp.pause_seconds.min(600)), &cancel)
                    .await?;
            }
            let units_here: Vec<usize> = mail.iter().map(|&k| ready[k].unit).collect();
            let positions: Vec<usize> = units_here
                .iter()
                .flat_map(|&u| unit_pos[u].iter().copied())
                .collect();
            let fail_all = |msg: String| {
                st.jobs.update_items(job_id, &positions, |it| {
                    it.state = "failed".into();
                    it.detail = msg.clone();
                });
            };
            // the daily limit is checked per mail (other jobs may have sent in the meantime)
            let (uid, day) = (user_id, local_today());
            let used = st.db.run(move |c| db::mail_count(c, uid, &day)).await?;
            if used >= i64::from(smtp.daily_limit_per_user) {
                fail_all(format!(
                    "daily mail limit reached ({} mails)",
                    smtp.daily_limit_per_user
                ));
                for &u in &units_here {
                    failed_units[u] = true;
                }
                continue;
            }
            st.jobs.running(
                job_id,
                0.6 + 0.4 * m as f64 / n_mails as f64,
                &format!("Sending e-mail {} of {n_mails}", m + 1),
            );
            let mut attachments = Vec::with_capacity(mail.len());
            for &k in mail {
                let r = &ready[k];
                attachments.push((
                    r.name.clone(),
                    r.mime.clone(),
                    tokio::fs::read(&r.path).await?,
                ));
            }
            let first = &units[units_here[0]].books[0].book;
            let subject = if mail.len() == 1 {
                let authors = first
                    .authors
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                smtp.subject_for(&first.title, &authors)
            } else {
                let s = format!("{} (+{} more)", first.title, mail.len() - 1);
                smtp.subject_for(&s, "")
            };
            let mut delay = Duration::from_secs(smtp.retry_delay_seconds);
            let mut attempt = 0u32;
            loop {
                attempt += 1;
                st.jobs.update_items(job_id, &positions, |it| {
                    it.state = "sending".into();
                    it.attempts += 1;
                    it.mail = Some(m as u32 + 1);
                    it.detail = format!("handed to {}", smtp.host.trim());
                });
                match crate::mail::deliver(
                    &smtp,
                    &to,
                    &subject,
                    "Sent by freeLib",
                    attachments.clone(),
                )
                .await
                {
                    Ok(reply) => {
                        let day = local_today();
                        st.db.run(move |c| db::add_mail(c, user_id, &day)).await?;
                        st.jobs.update_items(job_id, &positions, |it| {
                            it.state = "accepted".into();
                            it.detail = reply.clone();
                        });
                        for &u in &units_here {
                            ok_units[u] = true;
                        }
                        mails_sent += 1;
                        st.jobs.log(
                            job_id,
                            &format!("E-mail {} ({}): {reply}", m + 1, plural(mail.len(), "book")),
                        );
                        break;
                    }
                    Err(e) if !e.permanent && attempt <= smtp.retries => {
                        let msg = format!(
                            "{} — retry {attempt} of {} in {}s",
                            e.message,
                            smtp.retries,
                            delay.as_secs()
                        );
                        st.jobs.update_items(job_id, &positions, |it| {
                            it.state = "retrying".into();
                            it.detail = msg.clone();
                        });
                        st.jobs.log(job_id, &format!("E-mail {}: {msg}", m + 1));
                        sleep_cancellable(delay, &cancel).await?;
                        delay = (delay * 4).min(Duration::from_secs(3600));
                    }
                    Err(e) => {
                        fail_all(e.message.clone());
                        for &u in &units_here {
                            failed_units[u] = true;
                        }
                        st.jobs
                            .log(job_id, &format!("E-mail {}: {}", m + 1, e.message));
                        break;
                    }
                }
            }
        }
        if mails_sent > 0 && is_kindle_address(&to) {
            let from = crate::mail::from_address(&smtp).to_string();
            hint = Some(JobHint {
                code: "kindle_approved_sender".into(),
                text: format!(
                    "If it doesn't arrive in a few minutes, check that {from} is in Amazon's Approved Personal Document E-mail List ({KINDLE_APPROVED_URL})"
                ),
                from,
                url: KINDLE_APPROVED_URL.into(),
            });
        }
        let _ = tokio::fs::remove_dir_all(&job_dir).await;
    }

    let ok = ok_units.iter().filter(|x| **x).count();
    let failed = failed_units.iter().filter(|x| **x).count();
    if ok == 0 {
        let why = if dev.kind == "email" && failed > 0 {
            // the first error explains most failures
            st.jobs
                .items(job_id)
                .iter()
                .find(|i| i.state == "failed")
                .map(|i| format!(": {}", i.detail))
                .unwrap_or_default()
        } else {
            String::new()
        };
        return Err(ApiError::internal(format!(
            "all {} failed{why}",
            plural(total, "item")
        )));
    }
    let mut summary = if failed > 0 {
        format!("{ok} of {total} done, {failed} failed")
    } else {
        format!("{total} done")
    };
    if dev.kind == "email" && mails_sent > 0 {
        summary = if failed > 0 {
            format!(
                "{ok} of {total} sent in {}, {failed} failed",
                plural(mails_sent, "e-mail")
            )
        } else {
            format!(
                "{} sent in {}",
                plural(total, "book"),
                plural(mails_sent, "e-mail")
            )
        };
    }
    if hint.is_some() {
        st.jobs.set_hint(job_id, hint);
    }
    if dev.kind == "download" {
        let file = if produced.len() == 1 {
            let (name, p) = produced.remove(0);
            let ext = name.rsplit('.').next().unwrap_or("").to_string();
            JobFile {
                path: p,
                mime: mime_for_ext(&ext).into(),
                name,
            }
        } else {
            let zip_path = job_dir.join("books.zip");
            let zp = zip_path.clone();
            let items = produced.clone();
            tokio::task::spawn_blocking(move || write_zip(&zp, &items)).await??;
            for (_, p) in &produced {
                let _ = tokio::fs::remove_file(p).await;
            }
            JobFile {
                path: zip_path,
                mime: "application/zip".into(),
                name: format!("freeLib - {} books.zip", produced.len()),
            }
        };
        st.jobs.done(job_id, &summary, Some(file));
    } else {
        st.jobs.done(job_id, &summary, None);
    }
    let ok_ids = units
        .iter()
        .zip(&ok_units)
        .filter(|(_, ok)| **ok)
        .flat_map(|(u, _)| u.books.iter().map(|b| b.book.id))
        .collect();
    Ok(ok_ids)
}

fn make_units(details: Vec<BookDetail>, format: &str, opts: &ConvertOptions) -> Vec<Unit> {
    if !opts.join_series || format == "original" {
        return details
            .into_iter()
            .map(|b| Unit { books: vec![b] })
            .collect();
    }
    let mut out: Vec<Unit> = Vec::new();
    let mut by_series: HashMap<i64, usize> = HashMap::new();
    for b in details {
        let sid = b.book.series.as_ref().map(|s| s.id);
        match sid {
            Some(s) if b.book.ext == "fb2" => {
                if let Some(&i) = by_series.get(&s) {
                    out[i].books.push(b);
                } else {
                    by_series.insert(s, out.len());
                    out.push(Unit { books: vec![b] });
                }
            }
            _ => out.push(Unit { books: vec![b] }),
        }
    }
    for u in &mut out {
        u.books.sort_by_key(|b| b.book.serno.unwrap_or(i64::MAX));
    }
    out
}

/// Result and extension of one unit.
async fn produce_unit(
    st: &AppState,
    lib: i64,
    lib_dir: &std::path::Path,
    unit: &Unit,
    format: &str,
    opts: &ConvertOptions,
    cancel: &Arc<AtomicBool>,
) -> ApiResult<(Produced, String)> {
    let first = &unit.books[0];
    if unit.books.len() == 1 {
        let p: Produced =
            output::produce(st, lib, lib_dir, first, format, opts, Some(cancel)).await?;
        return Ok((p, output::file_ext(format, &first.book.ext)));
    }
    let _permit = st
        .workers
        .acquire()
        .await
        .map_err(|_| ApiError::internal("worker pool closed"))?;
    let mut sources = Vec::new();
    for b in &unit.books {
        let (dir, d) = (lib_dir.to_path_buf(), b.clone());
        sources.push(
            tokio::task::spawn_blocking(move || crate::bookio::read_original(&dir, &d)).await??,
        );
    }
    let title = first.book.series.as_ref().map(|s| s.name.clone());
    let conv = st.conv.clone();
    let o = opts.clone();
    let meta = freelib_fb2conv::BookMeta {
        book_key: Some(
            unit.books
                .iter()
                .map(|b| b.book.key.as_str())
                .collect::<Vec<_>>()
                .join("+"),
        ),
        lang: Some(first.book.lang.clone()),
        series: None,
        serno: None,
    };
    let epub = tokio::task::spawn_blocking(move || {
        let refs: Vec<&[u8]> = sources.iter().map(|v| v.as_slice()).collect();
        conv.join_to_epub(&refs, &o, title.as_deref(), &meta)
    })
    .await?
    .map_err(|e| ApiError::internal(format!("conversion failed: {e:#}")))?;
    match format {
        "epub" => Ok((Produced::Bytes(epub), "epub".into())),
        "kepub" => {
            let conv = st.conv.clone();
            let k = tokio::task::spawn_blocking(move || conv.to_kepub(&epub))
                .await?
                .map_err(|e| ApiError::internal(format!("KEPUB conversion failed: {e:#}")))?;
            Ok((Produced::Bytes(k), "kepub.epub".into()))
        }
        f => Ok((
            Produced::Bytes(output::calibre_run(st, &epub, "epub", f, Some(cancel)).await?),
            f.to_string(),
        )),
    }
}

fn write_zip(path: &std::path::Path, items: &[(String, PathBuf)]) -> ApiResult<()> {
    let f = std::fs::File::create(path)?;
    let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut used = std::collections::HashSet::new();
    for (name, p) in items {
        let mut n = name.clone();
        let mut k = 2;
        while !used.insert(n.clone()) {
            let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
            n = format!("{stem} ({k}).{ext}");
            k += 1;
        }
        let text = [".fb2", ".txt", ".html", ".htm", ".rtf"]
            .iter()
            .any(|e| n.to_lowercase().ends_with(e));
        z.start_file(n, if text { deflated } else { stored })
            .map_err(|e| ApiError::internal(format!("zip: {e}")))?;
        std::io::copy(&mut std::fs::File::open(p)?, &mut z)?;
    }
    z.finish()
        .map_err(|e| ApiError::internal(format!("zip: {e}")))?
        .flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn mails_respect_count_and_size() {
        // 60 small books: 25 + 25 + 10
        let (m, big) = plan_mails(&vec![MB; 60], 25, 50 * MB);
        assert_eq!(m.iter().map(Vec::len).collect::<Vec<_>>(), [25, 25, 10]);
        assert!(big.is_empty());
        assert_eq!(m.concat(), (0..60).collect::<Vec<_>>(), "order kept");
        // size bound: 20 + 20 > 30 → separate mails; 60 is too big for any mail
        let (m, big) = plan_mails(&[20 * MB, 20 * MB, 60 * MB, 5 * MB, 5 * MB], 25, 30 * MB);
        assert_eq!(m, vec![vec![0], vec![1, 3, 4]]);
        assert_eq!(big, vec![2]);
        // exactly at the limit fits
        let (m, _) = plan_mails(&[25 * MB, 25 * MB], 25, 50 * MB);
        assert_eq!(m, vec![vec![0, 1]]);
        assert_eq!(plan_mails(&[], 25, MB), (vec![], vec![]));
        // zero attachments configured still sends one per mail
        assert_eq!(plan_mails(&[1, 1], 0, MB).0.len(), 2);
    }

    #[test]
    fn encoded_sizes() {
        // base64 grows by 4/3 plus line breaks
        let e = encoded_size(3 * MB);
        assert!(e > 4 * MB && e < 4 * MB + 200 * 1024, "{e}");
        // 50 MB of attachments = about 37 MB of books
        let (m, _) = plan_mails(&[encoded_size(20 * MB), encoded_size(20 * MB)], 25, 50 * MB);
        assert_eq!(m.len(), 2);
        assert!(is_kindle_address("Me@Kindle.com") && is_kindle_address("x@free.kindle.com"));
        assert!(!is_kindle_address("me@example.com"));
    }
}
