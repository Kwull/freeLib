//! Producing book files: original bytes, EPUB/KEPUB via fb2conv, AZW3/MOBI/PDF via Calibre,
//! with a disk cache under `cache/out/<lib>/<bookhash>-<profilehash>.<ext>`.

use std::path::{Path, PathBuf};

use freelib_catalog::BookDetail;
use freelib_fb2conv::{ConvertOptions, NameFields};

use crate::bookio;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::short_hash;

pub const ALL_FORMATS: [&str; 6] = ["original", "epub", "kepub", "azw3", "mobi", "pdf"];
const CALIBRE_FORMATS: [&str; 3] = ["azw3", "mobi", "pdf"];
/// Non-FB2/EPUB inputs Calibre converts reasonably.
const CALIBRE_INPUTS: [&str; 11] = ["txt", "rtf", "html", "htm", "doc", "docx", "odt", "mobi", "azw3", "azw", "prc"];

/// Formats this server can produce for a book with extension `ext`.
pub fn formats_for(ext: &str, calibre: bool) -> Vec<String> {
    let mut v = vec!["original"];
    match ext {
        "fb2" | "epub" => {
            v.extend(["epub", "kepub"]);
            if calibre {
                v.extend(CALIBRE_FORMATS);
            }
        }
        e if calibre && CALIBRE_INPUTS.contains(&e) => {
            v.extend(["epub", "kepub"]);
            v.extend(CALIBRE_FORMATS.iter().filter(|f| **f != e));
        }
        _ => {}
    }
    v.into_iter().map(String::from).collect()
}

/// File extension of a produced format (`kepub` → `kepub.epub`).
pub fn file_ext(format: &str, orig_ext: &str) -> String {
    match format {
        "original" => orig_ext.to_string(),
        "kepub" => "kepub.epub".into(),
        f => f.into(),
    }
}

pub enum Produced {
    Bytes(Vec<u8>),
    File(PathBuf),
}

impl Produced {
    pub async fn into_bytes(self) -> ApiResult<Vec<u8>> {
        match self {
            Produced::Bytes(b) => Ok(b),
            Produced::File(p) => Ok(tokio::fs::read(&p).await?),
        }
    }
}

/// Validates `format` for the book; 501 when it would need Calibre and Calibre is missing.
pub fn check_format(st: &AppState, ext: &str, format: &str) -> ApiResult<()> {
    if !ALL_FORMATS.contains(&format) {
        return Err(ApiError::bad_request(format!("unknown format '{format}'")));
    }
    if formats_for(ext, st.calibre.is_some()).iter().any(|f| f == format) {
        return Ok(());
    }
    if formats_for(ext, true).iter().any(|f| f == format) {
        return Err(ApiError::unsupported(format!("{format} needs Calibre, which is not installed")));
    }
    Err(ApiError::unsupported(format!("cannot convert {ext} to {format}")))
}

fn book_hash(d: &BookDetail) -> String {
    short_hash(format!("{}\0{}\0{}", d.book.key, d.book.size, d.book.ext).as_bytes())
}

fn profile_hash(st: &AppState, format: &str, opts: &ConvertOptions) -> String {
    let calibre = st.calibre.as_ref().and_then(|c| c.version.clone()).unwrap_or_default();
    let o = serde_json::to_string(opts).unwrap_or_default();
    short_hash(format!("{format}\0{o}\0{}\0{calibre}", st.conv.version()).as_bytes())[..12].to_string()
}

async fn read_original(lib_dir: PathBuf, d: BookDetail) -> ApiResult<Vec<u8>> {
    tokio::task::spawn_blocking(move || bookio::read_original(&lib_dir, &d)).await?
}

async fn write_atomic(path: &Path, data: &[u8]) -> ApiResult<()> {
    if let Some(p) = path.parent() {
        tokio::fs::create_dir_all(p).await?;
    }
    let tmp = path.with_extension(format!("tmp{}", crate::util::random_id()));
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// Produces `format` of book `d` (library folder `lib_dir`) with `opts`.
pub async fn produce(
    st: &AppState,
    lib_id: i64,
    lib_dir: &Path,
    d: &BookDetail,
    format: &str,
    opts: &ConvertOptions,
) -> ApiResult<Produced> {
    let ext = d.book.ext.as_str();
    check_format(st, ext, format)?;
    if format == "original" || (format == "epub" && ext == "epub") {
        return Ok(Produced::Bytes(read_original(lib_dir.to_path_buf(), d.clone()).await?));
    }
    let out_dir = st.cache_dir("out", lib_id);
    let target = out_dir.join(format!("{}-{}.{}", book_hash(d), profile_hash(st, format, opts), file_ext(format, ext)));
    if target.is_file() {
        return Ok(Produced::File(target));
    }
    let _permit = st.workers.acquire().await.map_err(|_| ApiError::internal("worker pool closed"))?;
    if target.is_file() {
        return Ok(Produced::File(target));
    }
    match format {
        "epub" => {
            let data = if ext == "fb2" {
                let bytes = read_original(lib_dir.to_path_buf(), d.clone()).await?;
                let conv = st.conv.clone();
                let o = opts.clone();
                tokio::task::spawn_blocking(move || conv.fb2_to_epub(&bytes, &o))
                    .await?
                    .map_err(|e| ApiError::internal(format!("conversion failed: {e:#}")))?
            } else {
                calibre_convert(st, lib_dir, d, "epub").await?
            };
            write_atomic(&target, &data).await?;
        }
        "kepub" => {
            let epub = epub_bytes(st, lib_id, lib_dir, d, opts).await?;
            let conv = st.conv.clone();
            let data = tokio::task::spawn_blocking(move || conv.to_kepub(&epub))
                .await?
                .map_err(|e| ApiError::internal(format!("KEPUB conversion failed: {e:#}")))?;
            write_atomic(&target, &data).await?;
        }
        f => {
            let data = if ext == "fb2" {
                let epub = epub_bytes(st, lib_id, lib_dir, d, opts).await?;
                calibre_run(st, &epub, "epub", f).await?
            } else {
                calibre_convert(st, lib_dir, d, f).await?
            };
            write_atomic(&target, &data).await?;
        }
    }
    Ok(Produced::File(target))
}

/// EPUB bytes of the book (cached conversion for FB2, original for EPUB). Caller holds a permit.
async fn epub_bytes(st: &AppState, lib_id: i64, lib_dir: &Path, d: &BookDetail, opts: &ConvertOptions) -> ApiResult<Vec<u8>> {
    let ext = d.book.ext.as_str();
    if ext == "epub" {
        return read_original(lib_dir.to_path_buf(), d.clone()).await;
    }
    if ext != "fb2" {
        return calibre_convert(st, lib_dir, d, "epub").await;
    }
    let cached = st
        .cache_dir("out", lib_id)
        .join(format!("{}-{}.epub", book_hash(d), profile_hash(st, "epub", opts)));
    if let Ok(b) = tokio::fs::read(&cached).await {
        return Ok(b);
    }
    let bytes = read_original(lib_dir.to_path_buf(), d.clone()).await?;
    let conv = st.conv.clone();
    let o = opts.clone();
    let epub = tokio::task::spawn_blocking(move || conv.fb2_to_epub(&bytes, &o))
        .await?
        .map_err(|e| ApiError::internal(format!("conversion failed: {e:#}")))?;
    write_atomic(&cached, &epub).await?;
    Ok(epub)
}

async fn calibre_convert(st: &AppState, lib_dir: &Path, d: &BookDetail, to: &str) -> ApiResult<Vec<u8>> {
    let input = read_original(lib_dir.to_path_buf(), d.clone()).await?;
    calibre_run(st, &input, &d.book.ext, to).await
}

/// Runs Calibre on `input` (with extension `from`) and returns the output bytes.
pub async fn calibre_run(st: &AppState, input: &[u8], from: &str, to: &str) -> ApiResult<Vec<u8>> {
    let calibre = st.calibre.as_ref().ok_or_else(|| ApiError::unsupported(format!("{to} needs Calibre")))?;
    let tmp = st.cfg.cache_dir.join("tmp").join(crate::util::random_id());
    tokio::fs::create_dir_all(&tmp).await?;
    let from = if from.chars().all(|c| c.is_ascii_alphanumeric()) && !from.is_empty() { from } else { "bin" };
    let inp = tmp.join(format!("in.{from}"));
    let out = tmp.join(format!("out.{to}"));
    let r = async {
        tokio::fs::write(&inp, input).await?;
        calibre.convert(&inp, &out, st.cfg.calibre_timeout).await?;
        Ok::<_, ApiError>(tokio::fs::read(&out).await?)
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    r
}

/// Template fields of a catalog book.
pub fn name_fields(d: &freelib_catalog::Book) -> NameFields {
    let first = d.authors.first().map(|a| a.name.as_str()).unwrap_or("");
    let mut parts = first.split_whitespace();
    let last = parts.next().unwrap_or("").to_string();
    let firstname = parts.next().unwrap_or("").to_string();
    let middle = parts.collect::<Vec<_>>().join(" ");
    NameFields {
        author_last: last,
        author_first: firstname,
        author_middle: middle,
        series: d.series.as_ref().map(|s| s.name.clone()),
        serno: d.serno.and_then(|n| u32::try_from(n).ok()),
        title: d.title.clone(),
        lang: d.lang.clone(),
        date: d.date.clone(),
    }
}

/// Download file name: template (default `%a - %s %n - %b`) + extension, `/` flattened to ` - `
/// when `flat` (single download), kept as sub-folders otherwise.
pub fn download_name(st: &AppState, template: &str, b: &freelib_catalog::Book, format: &str, transliterate: bool, flat: bool) -> String {
    let base = st.conv.file_name(template, &name_fields(b), transliterate);
    let base = if flat { base.replace('/', " - ") } else { base };
    format!("{base}.{}", file_ext(format, &b.ext))
}
