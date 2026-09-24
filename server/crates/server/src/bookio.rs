//! Reading original book bytes: direct seek via `arch_offset`, zip fallback, plain files.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use freelib_catalog::BookDetail;

use crate::error::ApiError;

/// Largest book we read into memory.
const MAX_BOOK: u64 = 512 * 1024 * 1024;

fn safe_rel(rel: &str) -> Result<PathBuf, ApiError> {
    let p = PathBuf::from(rel.replace('\\', "/"));
    if p.as_os_str().is_empty()
        || p.components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(ApiError::forbidden("invalid book path"));
    }
    Ok(p)
}

fn not_found(what: &Path) -> ApiError {
    ApiError::not_found(format!(
        "book file not found: {}",
        what.file_name().unwrap_or_default().to_string_lossy()
    ))
}

/// Reads the original file of `d` from the library folder `lib_dir`. Blocking.
pub fn read_original(lib_dir: &Path, d: &BookDetail) -> Result<Vec<u8>, ApiError> {
    let rel = safe_rel(&d.relative_path())?;
    let path = lib_dir.join(&rel);
    if d.archive.is_empty() {
        let mut f = File::open(&path).map_err(|_| not_found(&path))?;
        let len = f.metadata()?.len();
        if len > MAX_BOOK {
            return Err(ApiError::bad_request("book file too large"));
        }
        let mut v = Vec::with_capacity(len as usize);
        f.read_to_end(&mut v)?;
        return Ok(v);
    }
    let mut f = File::open(&path).map_err(|_| not_found(&path))?;
    if let (Some(off), Some(csize), Some(method)) = (d.arch_offset, d.arch_csize, d.arch_method)
        && let Some(v) = read_at_offset(&mut f, off as u64, csize as u64, method, d.book.size)
    {
        return Ok(v);
    }
    // fallback: central directory lookup
    f.seek(SeekFrom::Start(0))?;
    let mut za = zip::ZipArchive::new(f)
        .map_err(|e| ApiError::internal(format!("zip {}: {e}", d.archive)))?;
    let name = d.entry_name();
    let idx = match za.index_for_name(&name) {
        Some(i) => i,
        // some archives store names with different case or inside folders
        None => (0..za.len())
            .find(|&i| {
                za.name_for_index(i).is_some_and(|n| {
                    n.eq_ignore_ascii_case(&name) || n.rsplit('/').next() == Some(name.as_str())
                })
            })
            .ok_or_else(|| ApiError::not_found(format!("{name} not found in {}", d.archive)))?,
    };
    let mut entry = za
        .by_index(idx)
        .map_err(|e| ApiError::internal(format!("zip: {e}")))?;
    if entry.size() > MAX_BOOK {
        return Err(ApiError::bad_request("book file too large"));
    }
    let mut v = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut v)
        .map_err(|e| ApiError::internal(format!("zip read: {e}")))?;
    Ok(v)
}

fn read_at_offset(
    f: &mut File,
    header: u64,
    csize: u64,
    method: i64,
    size_hint: i64,
) -> Option<Vec<u8>> {
    if csize > MAX_BOOK {
        return None;
    }
    let start = freelib_import::zipdir::local_data_start(f, header).ok()?;
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut raw = vec![0u8; csize as usize];
    f.read_exact(&mut raw).ok()?;
    match method {
        0 => Some(raw),
        8 => {
            let mut out = Vec::with_capacity(size_hint.clamp(0, 64 << 20) as usize);
            flate2::read::DeflateDecoder::new(&raw[..])
                .take(MAX_BOOK)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        }
        _ => None,
    }
}
