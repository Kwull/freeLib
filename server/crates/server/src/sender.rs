//! `POST /send` jobs: e-mail (Send to Kindle), export to a server folder, download (file or zip).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use freelib_catalog::BookDetail;
use freelib_fb2conv::ConvertOptions;

use crate::db::{self, Device, SmtpConfig, User};
use crate::error::{ApiError, ApiResult};
use crate::jobs::{Job, JobFile};
use crate::output::{self, Produced};
use crate::state::AppState;
use crate::util::{join_safe, mime_for_ext, safe_subdir};

pub const MAX_BOOKS: usize = 1000;

pub struct SendRequest {
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

pub async fn start(st: &AppState, user: &User, req: SendRequest) -> ApiResult<Job> {
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
    let smtp = match dev.kind.as_str() {
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
            if used + req.books.len() as i64 > limit {
                return Err(ApiError::rate_limited(format!(
                    "daily mail limit reached: {used} of {limit} mails sent today"
                )));
            }
            Some(smtp)
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
            None
        }
        "download" => None,
        _ => return Err(ApiError::bad_request("unknown device kind")),
    };
    st.catalog(req.library)?;
    let n = req.books.len();
    let (kind, title) = match dev.kind.as_str() {
        "email" => (
            "send",
            format!(
                "Send to {} · {n} book{}",
                dev.name,
                if n == 1 { "" } else { "s" }
            ),
        ),
        "folder" => (
            "export",
            format!(
                "Export to {} · {n} book{}",
                dev.name,
                if n == 1 { "" } else { "s" }
            ),
        ),
        _ => (
            "download",
            format!(
                "Download for {} · {n} book{}",
                dev.name,
                if n == 1 { "" } else { "s" }
            ),
        ),
    };
    let max = st.cfg.max_jobs_per_user;
    let (job, cancel) = st
        .jobs
        .try_create(kind, &title, user.id, max)
        .ok_or_else(|| {
            ApiError::rate_limited(format!(
                "at most {max} send/download jobs can run at once; wait for one to finish"
            ))
        })?;
    let st2 = st.clone();
    let job_id = job.id.clone();
    let uid = user.id;
    let n = req.books.len().min(crate::extrating::BROWSE_ENQUEUE);
    st.ext.enqueue(
        crate::extrating::Priority::User,
        req.library,
        &req.books[..n],
    );
    let (lib, ids) = (req.library, req.books.clone());
    let action = if dev.kind == "download" {
        "download"
    } else {
        "send"
    };
    let dev_name = dev.name.clone();
    tokio::spawn(async move {
        let r = run(&st2, &job_id, uid, req, target, smtp, cancel).await;
        if r.is_ok() {
            record_history(&st2, uid, lib, ids, action, dev_name).await;
        }
        match r {
            Ok(()) => {}
            Err(e) if e.code == "cancelled" => st2.jobs.cancelled(&job_id),
            Err(e) => st2.jobs.fail(&job_id, &e.message),
        }
    });
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

#[allow(clippy::too_many_arguments)]
async fn run(
    st: &AppState,
    job_id: &str,
    user_id: i64,
    req: SendRequest,
    target: Option<String>,
    smtp: Option<SmtpConfig>,
    cancel: Arc<AtomicBool>,
) -> ApiResult<()> {
    let dev = req.device;
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
    let total = units.len();
    let multi_file = total > 1;
    let job_dir = st.cfg.cache_dir.join("jobs").join(job_id);
    let mut produced: Vec<(String, PathBuf)> = Vec::new();
    let mut errors = 0usize;
    for (i, unit) in units.iter().enumerate() {
        if st.jobs.is_cancelled(job_id) {
            return Err(cancelled());
        }
        let first = &unit.books[0];
        st.jobs
            .running(job_id, i as f64 / total as f64, &first.book.title);
        let res = produce_unit(st, req.library, &lib_dir, unit, &dev.format, &opts, &cancel).await;
        let (data, ext) = match res {
            Ok(x) => x,
            Err(e) if e.code == "cancelled" => return Err(cancelled()),
            Err(e) => {
                errors += 1;
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
                let smtp = smtp
                    .as_ref()
                    .ok_or_else(|| ApiError::internal("smtp missing"))?;
                if i > 0 && smtp.pause_seconds > 0 {
                    tokio::time::sleep(Duration::from_secs(smtp.pause_seconds.min(600))).await;
                }
                let name = format!("{}.{ext}", base.replace('/', " - "));
                let to = target.clone().unwrap_or_default();
                let mime = mime_for_ext(ext.rsplit('.').next().unwrap_or(&ext));
                let data = data.into_bytes().await?;
                let day = local_today();
                st.db.run(move |c| db::add_mail(c, user_id, &day)).await?;
                let authors = book
                    .authors
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let subject = smtp.subject_for(&book.title, &authors);
                match crate::mail::send(
                    smtp,
                    &to,
                    &subject,
                    "Sent by freeLib",
                    Some((&name, mime, data)),
                )
                .await
                {
                    Ok(()) => st.jobs.log(job_id, &format!("Sent {name}")),
                    Err(e) => {
                        errors += 1;
                        st.jobs.log(job_id, &format!("{name}: {}", e.message));
                    }
                }
            }
            "folder" => {
                let sub = safe_subdir(target.as_deref().unwrap_or("")).unwrap_or_default();
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
                produced.push((rel, p));
            }
        }
    }
    if errors == total {
        return Err(ApiError::internal(format!("all {total} items failed")));
    }
    let summary = if errors > 0 {
        format!("{} of {total} done, {errors} failed", total - errors)
    } else {
        format!("{total} done")
    };
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
    Ok(())
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
    let epub = tokio::task::spawn_blocking(move || {
        let refs: Vec<&[u8]> = sources.iter().map(|v| v.as_slice()).collect();
        conv.join_to_epub(&refs, &o, title.as_deref())
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
