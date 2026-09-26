//! Reading original book bytes: direct seek via `arch_offset`, zip fallback, plain files.
//!
//! Every read is bounded by [`MAX_BOOK`]: sizes from the catalog or a zip header only size
//! buffers and are validated against the file, and a deflated entry is inflated through a
//! reader that fails past the limit (the zip crate does not enforce declared sizes).

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use freelib_catalog::BookDetail;
use freelib_import::zipdir::{entry_matches, local_data_start};

use crate::error::ApiError;

/// Largest book we serve or convert (uncompressed).
pub const MAX_BOOK: u64 = 256 * 1024 * 1024;

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

fn too_large() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::PAYLOAD_TOO_LARGE,
        "bad_request",
        format!("book file larger than {} MiB", MAX_BOOK >> 20),
    )
}

/// Maps a read error of a book stream to an API error.
pub fn read_error(e: io::Error) -> ApiError {
    if e.kind() == io::ErrorKind::InvalidData && e.to_string().contains("limit") {
        too_large()
    } else {
        ApiError::internal(format!("book read: {e}"))
    }
}

/// A reader that fails with `InvalidData` once more than `left` bytes would be produced.
pub struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Capped<R> {
    pub fn new(inner: R, limit: u64) -> Capped<R> {
        Capped { inner, left: limit }
    }
}

impl<R: Read> Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let mut probe = [0u8; 1];
            return match self.inner.read(&mut probe)? {
                0 => Ok(0),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "book exceeds the size limit",
                )),
            };
        }
        let n = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let k = self.inner.read(&mut buf[..n])?;
        self.left -= k as u64;
        Ok(k)
    }
}

/// The original file of a book as a bounded stream.
pub struct Original {
    pub reader: Box<dyn Read + Send>,
    /// Exact length when known up front (plain files and stored zip entries).
    pub len: Option<u64>,
}

/// Where an entry's data lives inside an archive.
struct EntryData {
    start: u64,
    csize: u64,
    method: u16,
}

/// Validates catalog/header values against the archive file itself.
fn locate(f: &mut File, file_len: u64, header: u64, csize: u64, method: u16) -> Option<EntryData> {
    if header >= file_len || !matches!(method, 0 | 8) {
        return None;
    }
    let start = local_data_start(f, header).ok()?;
    if start.checked_add(csize).is_none_or(|end| end > file_len) {
        return None;
    }
    if method == 0 && csize > MAX_BOOK {
        return None;
    }
    Some(EntryData {
        start,
        csize,
        method,
    })
}

/// Central-directory lookup with the importer's name rules
/// ([`freelib_import::zipdir::entry_matches`]): exact name, then case-insensitive full name,
/// then case-insensitive last path component.
fn locate_by_name(f: File, file_len: u64, d: &BookDetail) -> Result<(File, EntryData), ApiError> {
    let mut za = zip::ZipArchive::new(f)
        .map_err(|e| ApiError::internal(format!("zip {}: {e}", d.archive)))?;
    let name = d.entry_name();
    let idx = match za.index_for_name(&name) {
        Some(i) => i,
        None => {
            let mut cands: Vec<(u8, String, usize)> = (0..za.len())
                .filter_map(|i| {
                    let n = za.name_for_index(i)?;
                    if n.ends_with('/') || !entry_matches(n, &name) {
                        return None;
                    }
                    let rank = if n.eq_ignore_ascii_case(&name) { 0 } else { 1 };
                    Some((rank, n.to_string(), i))
                })
                .collect();
            cands.sort();
            cands
                .first()
                .map(|c| c.2)
                .ok_or_else(|| ApiError::not_found(format!("{name} not found in {}", d.archive)))?
        }
    };
    let (header, csize, method, encrypted) = {
        let e = za
            .by_index_raw(idx)
            .map_err(|e| ApiError::internal(format!("zip: {e}")))?;
        let method = match e.compression() {
            zip::CompressionMethod::Stored => 0u16,
            zip::CompressionMethod::Deflated => 8,
            _ => u16::MAX,
        };
        (e.header_start(), e.compressed_size(), method, e.encrypted())
    };
    if encrypted || method == u16::MAX {
        return Err(ApiError::unsupported(format!(
            "{name}: unsupported zip compression or encryption"
        )));
    }
    let mut f = za.into_inner();
    let loc = locate(&mut f, file_len, header, csize, method).ok_or_else(|| {
        if method == 0 && csize > MAX_BOOK {
            too_large()
        } else {
            ApiError::internal(format!("zip {}: bad entry {name}", d.archive))
        }
    })?;
    Ok((f, loc))
}

/// Opens the original file of `d` from the library folder `lib_dir`. Blocking.
pub fn open_original(lib_dir: &Path, d: &BookDetail) -> Result<Original, ApiError> {
    let rel = safe_rel(&d.relative_path())?;
    let path = lib_dir.join(&rel);
    let mut f = File::open(&path).map_err(|_| not_found(&path))?;
    let file_len = f.metadata()?.len();
    if d.archive.is_empty() {
        if file_len > MAX_BOOK {
            return Err(too_large());
        }
        return Ok(Original {
            reader: Box::new(f.take(file_len)),
            len: Some(file_len),
        });
    }
    let known = match (d.arch_offset, d.arch_csize, d.arch_method) {
        (Some(off), Some(cs), Some(m)) if off >= 0 && cs >= 0 => {
            locate(&mut f, file_len, off as u64, cs as u64, m as u16)
        }
        _ => None,
    };
    let (mut f, loc) = match known {
        Some(l) => (f, l),
        None => {
            f.seek(SeekFrom::Start(0))?;
            locate_by_name(f, file_len, d)?
        }
    };
    f.seek(SeekFrom::Start(loc.start))?;
    let raw = f.take(loc.csize);
    Ok(if loc.method == 0 {
        Original {
            reader: Box::new(raw),
            len: Some(loc.csize),
        }
    } else {
        let inflate = flate2::read::DeflateDecoder::new(BufReader::with_capacity(64 << 10, raw));
        Original {
            reader: Box::new(Capped::new(inflate, MAX_BOOK)),
            len: None,
        }
    })
}

/// Reads the original file of `d` into memory (at most [`MAX_BOOK`] bytes). Blocking.
pub fn read_original(lib_dir: &Path, d: &BookDetail) -> Result<Vec<u8>, ApiError> {
    let o = open_original(lib_dir, d)?;
    let hint = o.len.unwrap_or(d.book.size.max(0) as u64);
    freelib_fb2conv::limit::read_limited(o.reader, MAX_BOOK, hint).map_err(read_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use freelib_catalog::Book;
    use freelib_import::testutil::{raw_zip, zip_bomb};

    fn detail(archive: &str, loc: Option<(i64, i64, i64)>) -> BookDetail {
        BookDetail {
            book: Book {
                id: 1,
                key: "k".into(),
                title: "t".into(),
                authors: Vec::new(),
                series: None,
                serno: None,
                genres: Vec::new(),
                lang: "ru".into(),
                ext: "fb2".into(),
                size: 100,
                date: "2024-01-01".into(),
                deleted: false,
                lib_rating: 0,
                kids_age: None,
                editions: None,
            },
            file: "1".into(),
            archive: archive.into(),
            folder: String::new(),
            keywords: String::new(),
            lib_id: None,
            stars: 0,
            arch_offset: loc.map(|l| l.0),
            arch_csize: loc.map(|l| l.1),
            arch_method: loc.map(|l| l.2),
        }
    }

    #[test]
    fn zip_bomb_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mib = (MAX_BOOK >> 20) as usize + 1;
        let z = zip_bomb("1.fb2", mib, 100);
        // local header (30 + name) … central directory (46 + name) + end record (22)
        let csize = z.len() as i64 - 35 - 51 - 22;
        std::fs::write(dir.path().join("a.zip"), &z).unwrap();
        // via the zip central directory
        let e = read_original(dir.path(), &detail("a.zip", None)).unwrap_err();
        assert_eq!(e.status, axum::http::StatusCode::PAYLOAD_TOO_LARGE, "{e}");
        // via stored offsets (the header is at 0)
        let e = read_original(dir.path(), &detail("a.zip", Some((0, csize, 8)))).unwrap_err();
        assert_eq!(e.status, axum::http::StatusCode::PAYLOAD_TOO_LARGE, "{e}");
    }

    #[test]
    fn forged_sizes() {
        let dir = tempfile::tempdir().unwrap();
        // declared size of 4 EiB must not be preallocated
        std::fs::write(
            dir.path().join("a.zip"),
            raw_zip("1.fb2", 0, b"<FictionBook/>", 1 << 62),
        )
        .unwrap();
        let v = read_original(dir.path(), &detail("a.zip", None)).unwrap();
        assert_eq!(v, b"<FictionBook/>");
        // catalog offsets pointing past the file fall back to the central directory
        let v = read_original(dir.path(), &detail("a.zip", Some((0, 1 << 40, 0)))).unwrap();
        assert_eq!(v, b"<FictionBook/>");
        let v = read_original(dir.path(), &detail("a.zip", Some((1 << 40, 5, 0)))).unwrap();
        assert_eq!(v, b"<FictionBook/>");
    }

    #[test]
    fn case_insensitive_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.zip"),
            raw_zip("Folder/1.FB2", 0, b"x", 1),
        )
        .unwrap();
        assert_eq!(
            read_original(dir.path(), &detail("a.zip", None)).unwrap(),
            b"x"
        );
    }

    #[test]
    fn capped() {
        let data = [1u8; 100];
        let mut out = Vec::new();
        Capped::new(&data[..], 100).read_to_end(&mut out).unwrap();
        assert_eq!(out.len(), 100);
        let mut out = Vec::new();
        assert!(Capped::new(&data[..], 99).read_to_end(&mut out).is_err());
    }
}
