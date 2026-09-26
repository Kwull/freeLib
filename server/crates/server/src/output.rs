//! Producing book files: original bytes, EPUB/KEPUB via fb2conv, AZW3/MOBI/PDF via Calibre,
//! with a disk cache under `cache/out/<lib>/<bookhash>-<profilehash>.<ext>`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use freelib_catalog::BookDetail;
use freelib_fb2conv::{BookMeta, ConvertOptions, NameFields};

use crate::bookio;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::short_hash;

pub const ALL_FORMATS: [&str; 6] = ["original", "epub", "kepub", "azw3", "mobi", "pdf"];
const CALIBRE_FORMATS: [&str; 3] = ["azw3", "mobi", "pdf"];

/// Formats this server can produce for a book with extension `ext`.
///
/// Calibre only ever gets EPUB input (our own FB2 → EPUB conversion or an original `.epub`):
/// its other input plugins (HTML, TXT, DOCX, …) parse untrusted documents with a much larger
/// attack surface, so other formats are only served as originals.
pub fn formats_for(ext: &str, calibre: bool) -> Vec<String> {
    let mut v = vec!["original"];
    if matches!(ext, "fb2" | "epub") {
        v.extend(["epub", "kepub"]);
        if calibre {
            v.extend(CALIBRE_FORMATS);
        }
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
    /// The original file, streamed (never held in memory as a whole).
    Stream(bookio::Original),
}

impl Produced {
    pub async fn into_bytes(self) -> ApiResult<Vec<u8>> {
        match self {
            Produced::Bytes(b) => Ok(b),
            Produced::File(p) => Ok(tokio::fs::read(&p).await?),
            Produced::Stream(o) => {
                tokio::task::spawn_blocking(move || {
                    let hint = o.len.unwrap_or(0);
                    freelib_fb2conv::limit::read_limited(o.reader, bookio::MAX_BOOK, hint)
                        .map_err(bookio::read_error)
                })
                .await?
            }
        }
    }

    /// Writes the result to `path` (copy / stream, without loading files into memory).
    pub async fn write_to(self, path: &Path) -> ApiResult<()> {
        match self {
            Produced::Bytes(b) => Ok(tokio::fs::write(path, b).await?),
            Produced::File(p) => {
                tokio::fs::copy(&p, path).await?;
                Ok(())
            }
            Produced::Stream(o) => {
                let path = path.to_path_buf();
                tokio::task::spawn_blocking(move || -> ApiResult<()> {
                    let mut out = std::fs::File::create(&path)?;
                    let mut r = o.reader;
                    std::io::copy(&mut r, &mut out).map_err(bookio::read_error)?;
                    Ok(())
                })
                .await?
            }
        }
    }
}

async fn open_original(lib_dir: PathBuf, d: BookDetail) -> ApiResult<bookio::Original> {
    tokio::task::spawn_blocking(move || bookio::open_original(&lib_dir, &d)).await?
}

/// Marks a cached file as recently used (the cache is evicted by modification time).
fn touch(p: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(p) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// Validates `format` for the book; 501 when it would need Calibre and Calibre is missing.
pub fn check_format(st: &AppState, ext: &str, format: &str) -> ApiResult<()> {
    if !ALL_FORMATS.contains(&format) {
        return Err(ApiError::bad_request(format!("unknown format '{format}'")));
    }
    if formats_for(ext, st.calibre.is_some())
        .iter()
        .any(|f| f == format)
    {
        return Ok(());
    }
    if formats_for(ext, true).iter().any(|f| f == format) {
        return Err(ApiError::unsupported(format!(
            "{format} needs Calibre, which is not installed"
        )));
    }
    Err(ApiError::unsupported(format!(
        "cannot convert {ext} to {format}"
    )))
}

fn book_hash(d: &BookDetail) -> String {
    // the catalog metadata that goes into the file is part of the key
    let m = book_meta(&d.book);
    short_hash(
        format!(
            "{}\0{}\0{}\0{:?}\0{:?}\0{:?}",
            d.book.key, d.book.size, d.book.ext, m.lang, m.series, m.serno
        )
        .as_bytes(),
    )
}

/// Catalog metadata for a converted file: the book key (stable EPUB identifier), the
/// catalog language (fallback) and series.
pub fn book_meta(b: &freelib_catalog::Book) -> BookMeta {
    BookMeta {
        book_key: Some(b.key.clone()),
        lang: Some(b.lang.clone()).filter(|l| !l.trim().is_empty()),
        series: b.series.as_ref().map(|s| s.name.clone()),
        serno: b
            .serno
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0),
    }
}

fn profile_hash(st: &AppState, format: &str, opts: &ConvertOptions) -> String {
    let calibre = st
        .calibre
        .as_ref()
        .and_then(|c| c.version.clone())
        .unwrap_or_default();
    let o = serde_json::to_string(opts).unwrap_or_default();
    short_hash(format!("{format}\0{o}\0{}\0{calibre}", st.conv.version()).as_bytes())[..12]
        .to_string()
}

async fn read_original(lib_dir: PathBuf, d: BookDetail) -> ApiResult<Vec<u8>> {
    tokio::task::spawn_blocking(move || bookio::read_original(&lib_dir, &d)).await?
}

/// Suffix of the temporary files of [`write_atomic`] (`<name>.tmp<16 hex>`), removed at startup.
pub const TMP_EXT_PREFIX: &str = "tmp";

async fn write_atomic(st: &AppState, path: &Path, data: &[u8]) -> ApiResult<()> {
    if let Some(p) = path.parent() {
        tokio::fs::create_dir_all(p).await?;
    }
    let tmp = path.with_extension(format!("{TMP_EXT_PREFIX}{}", crate::util::random_id()));
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, path).await?;
    crate::cache::written(st, data.len() as u64);
    Ok(())
}

/// Produces `format` of book `d` (library folder `lib_dir`) with `opts`. `cancel` (a job's
/// cancellation flag) stops a running Calibre conversion.
pub async fn produce(
    st: &AppState,
    lib_id: i64,
    lib_dir: &Path,
    d: &BookDetail,
    format: &str,
    opts: &ConvertOptions,
    cancel: Option<&Arc<AtomicBool>>,
) -> ApiResult<Produced> {
    let ext = d.book.ext.as_str();
    check_format(st, ext, format)?;
    if format == "original" || (format == "epub" && ext == "epub") {
        return Ok(Produced::Stream(
            open_original(lib_dir.to_path_buf(), d.clone()).await?,
        ));
    }
    let out_dir = st.cache_dir("out", lib_id);
    let target = out_dir.join(format!(
        "{}-{}.{}",
        book_hash(d),
        profile_hash(st, format, opts),
        file_ext(format, ext)
    ));
    if target.is_file() {
        touch(&target);
        return Ok(Produced::File(target));
    }
    let _permit = st
        .workers
        .acquire()
        .await
        .map_err(|_| ApiError::internal("worker pool closed"))?;
    if target.is_file() {
        touch(&target);
        return Ok(Produced::File(target));
    }
    match format {
        "epub" => {
            // only FB2 gets here (EPUB originals are served as they are)
            let bytes = read_original(lib_dir.to_path_buf(), d.clone()).await?;
            let conv = st.conv.clone();
            let o = opts.clone();
            let meta = book_meta(&d.book);
            let data = tokio::task::spawn_blocking(move || conv.fb2_to_epub(&bytes, &o, &meta))
                .await?
                .map_err(|e| ApiError::internal(format!("conversion failed: {e:#}")))?;
            write_atomic(st, &target, &data).await?;
        }
        "kepub" => {
            let epub = epub_bytes(st, lib_id, lib_dir, d, opts).await?;
            let conv = st.conv.clone();
            let data = tokio::task::spawn_blocking(move || conv.to_kepub(&epub))
                .await?
                .map_err(|e| ApiError::internal(format!("KEPUB conversion failed: {e:#}")))?;
            write_atomic(st, &target, &data).await?;
        }
        f => {
            let epub = epub_bytes(st, lib_id, lib_dir, d, opts).await?;
            let data = calibre_run(st, &epub, "epub", f, cancel).await?;
            write_atomic(st, &target, &data).await?;
        }
    }
    Ok(Produced::File(target))
}

/// EPUB bytes of the book (cached conversion for FB2, original for EPUB). Caller holds a permit.
async fn epub_bytes(
    st: &AppState,
    lib_id: i64,
    lib_dir: &Path,
    d: &BookDetail,
    opts: &ConvertOptions,
) -> ApiResult<Vec<u8>> {
    let ext = d.book.ext.as_str();
    if ext == "epub" {
        return read_original(lib_dir.to_path_buf(), d.clone()).await;
    }
    if ext != "fb2" {
        return Err(ApiError::unsupported(format!(
            "cannot convert {ext} to EPUB"
        )));
    }
    let cached = st.cache_dir("out", lib_id).join(format!(
        "{}-{}.epub",
        book_hash(d),
        profile_hash(st, "epub", opts)
    ));
    if let Ok(b) = tokio::fs::read(&cached).await {
        touch(&cached);
        return Ok(b);
    }
    let bytes = read_original(lib_dir.to_path_buf(), d.clone()).await?;
    let conv = st.conv.clone();
    let o = opts.clone();
    let meta = book_meta(&d.book);
    let epub = tokio::task::spawn_blocking(move || conv.fb2_to_epub(&bytes, &o, &meta))
        .await?
        .map_err(|e| ApiError::internal(format!("conversion failed: {e:#}")))?;
    write_atomic(st, &cached, &epub).await?;
    Ok(epub)
}

/// Runs Calibre on `input` and returns the output bytes. Only EPUB input is accepted (see
/// [`formats_for`]); Calibre runs in a fresh temporary directory (its cwd and `HOME`) with
/// absolute paths we create ourselves.
pub async fn calibre_run(
    st: &AppState,
    input: &[u8],
    from: &str,
    to: &str,
    cancel: Option<&Arc<AtomicBool>>,
) -> ApiResult<Vec<u8>> {
    let calibre = st
        .calibre
        .as_ref()
        .ok_or_else(|| ApiError::unsupported(format!("{to} needs Calibre")))?;
    if from != "epub" || !CALIBRE_FORMATS.contains(&to) {
        return Err(ApiError::unsupported(format!(
            "Calibre converts EPUB only, not {from} to {to}"
        )));
    }
    let tmp_root = st.cfg.cache_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_root).await?;
    let tmp = tokio::fs::canonicalize(&tmp_root)
        .await?
        .join(crate::util::random_id());
    tokio::fs::create_dir_all(&tmp).await?;
    let inp = tmp.join("in.epub");
    let out = tmp.join(format!("out.{to}"));
    // the book's metadata and cover, passed explicitly so AZW3/MOBI get them in their EXTH
    // headers (Kindle's library, series grouping and thumbnails) whatever Calibre's EPUB
    // input makes of the package
    let conv = st.conv.clone();
    let epub = input.to_vec();
    let info = tokio::task::spawn_blocking(move || conv.read_info(&epub).ok()).await?;
    let r = async {
        tokio::fs::write(&inp, input).await?;
        let mut args = Vec::new();
        if let Some(info) = &info {
            args = crate::calibre::metadata_args(info);
            if let Some(c) = &info.cover {
                let ext = if c.mime == "image/png" { "png" } else { "jpg" };
                let cover = tmp.join(format!("cover.{ext}"));
                tokio::fs::write(&cover, &c.data).await?;
                args.push(format!("--cover={}", cover.display()));
            }
        }
        calibre
            .convert(&inp, &out, &tmp, &args, st.cfg.calibre_timeout, cancel)
            .await?;
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
pub fn download_name(
    st: &AppState,
    template: &str,
    b: &freelib_catalog::Book,
    format: &str,
    transliterate: bool,
    flat: bool,
) -> String {
    let base = st.conv.file_name(template, &name_fields(b), transliterate);
    let base = if flat { base.replace('/', " - ") } else { base };
    format!("{base}.{}", file_ext(format, &b.ext))
}
