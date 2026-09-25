//! Minimal zip central-directory reader: file name → local header offset, sizes, method.
//!
//! The `zip` crate reads each entry's local header when an entry is opened, which costs one
//! seek per book; for resolving offsets of whole archives (thousands of entries) we only need
//! the central directory, read here in a single pass. Zip64 is supported.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Location of one entry inside a zip file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipEntryLoc {
    /// Offset of the local file header.
    pub offset: u64,
    /// Compressed size.
    pub csize: u64,
    /// Uncompressed size.
    pub usize: u64,
    /// Compression method (0 = stored, 8 = deflate).
    pub method: u16,
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64le(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Read the central directory of `path`. Entry names are decoded as UTF-8 (lossy);
/// directory entries are skipped.
pub fn read_central_directory(path: &Path) -> io::Result<HashMap<String, ZipEntryLoc>> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    if len < 22 {
        return Err(bad("file too small for a zip"));
    }
    let tail_len = len.min(22 + 65_535 + 20);
    let mut tail = vec![0u8; tail_len as usize];
    f.seek(SeekFrom::Start(len - tail_len))?;
    f.read_exact(&mut tail)?;
    let eocd = (0..=tail.len() - 22)
        .rev()
        .find(|&i| u32le(&tail, i) == 0x0605_4b50)
        .ok_or_else(|| bad("end of central directory not found"))?;
    let mut entries = u16le(&tail, eocd + 10) as u64;
    let mut cd_size = u32le(&tail, eocd + 12) as u64;
    let mut cd_off = u32le(&tail, eocd + 16) as u64;
    if (entries == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF) && eocd >= 20 {
        let loc = eocd - 20;
        if u32le(&tail, loc) == 0x0706_4b50 {
            let z64_off = u64le(&tail, loc + 8);
            let mut rec = [0u8; 56];
            f.seek(SeekFrom::Start(z64_off))?;
            f.read_exact(&mut rec)?;
            if u32le(&rec, 0) != 0x0606_4b50 {
                return Err(bad("bad zip64 end of central directory"));
            }
            entries = u64le(&rec, 32);
            cd_size = u64le(&rec, 40);
            cd_off = u64le(&rec, 48);
        }
    }
    if cd_off.checked_add(cd_size).is_none_or(|end| end > len) {
        return Err(bad("central directory out of range"));
    }
    // ~100 bytes per entry: 512 MiB is millions of books, far beyond any real archive
    if cd_size > 512 * 1024 * 1024 {
        return Err(bad("central directory too large"));
    }
    let mut cd = vec![0u8; cd_size as usize];
    f.seek(SeekFrom::Start(cd_off))?;
    f.read_exact(&mut cd)?;

    let mut out = HashMap::with_capacity(entries.min(1 << 20) as usize);
    let mut p = 0usize;
    while p + 46 <= cd.len() && u32le(&cd, p) == 0x0201_4b50 {
        let method = u16le(&cd, p + 10);
        let mut csize = u32le(&cd, p + 20) as u64;
        let mut usize_ = u32le(&cd, p + 24) as u64;
        let name_len = u16le(&cd, p + 28) as usize;
        let extra_len = u16le(&cd, p + 30) as usize;
        let comment_len = u16le(&cd, p + 32) as usize;
        let mut offset = u32le(&cd, p + 42) as u64;
        let name_end = p + 46 + name_len;
        let extra_end = name_end + extra_len;
        if extra_end + comment_len > cd.len() {
            return Err(bad("truncated central directory entry"));
        }
        // Zip64 extended information: present fields are the ones saturated in the header.
        let mut e = name_end;
        while e + 4 <= extra_end {
            let id = u16le(&cd, e);
            let sz = u16le(&cd, e + 2) as usize;
            let data = e + 4;
            if id == 0x0001 && data + sz <= extra_end {
                let mut q = data;
                let mut take = |v: &mut u64| {
                    if *v == 0xFFFF_FFFF && q + 8 <= data + sz {
                        *v = u64le(&cd, q);
                        q += 8;
                    }
                };
                take(&mut usize_);
                take(&mut csize);
                take(&mut offset);
            }
            e = data + sz;
        }
        let name = String::from_utf8_lossy(&cd[p + 46..name_end]).into_owned();
        if !name.ends_with('/') {
            out.insert(
                name,
                ZipEntryLoc {
                    offset,
                    csize,
                    usize: usize_,
                    method,
                },
            );
        }
        p = extra_end + comment_len;
    }
    Ok(out)
}

/// Case-insensitive name match used whenever an INPX entry name is looked up in an archive:
/// the full entry name, or the entry's last path component (some archives keep books in a
/// folder), both ASCII-case-insensitively. The zip crate fallback of the server uses the same
/// rule, so offsets resolved at import time and the fallback agree.
pub fn entry_matches(entry: &str, wanted: &str) -> bool {
    entry.eq_ignore_ascii_case(wanted)
        || entry
            .rsplit('/')
            .next()
            .is_some_and(|b| b.eq_ignore_ascii_case(wanted))
}

/// A central directory with [`entry_matches`] lookups (exact name first).
pub struct EntryIndex {
    exact: HashMap<String, ZipEntryLoc>,
    /// lower-cased full name and lower-cased last component → first entry in name order
    folded: HashMap<String, ZipEntryLoc>,
}

impl EntryIndex {
    pub fn new(exact: HashMap<String, ZipEntryLoc>) -> EntryIndex {
        let mut names: Vec<&String> = exact.keys().collect();
        names.sort();
        let mut full = HashMap::with_capacity(exact.len());
        let mut base = HashMap::new();
        for n in names {
            let loc = exact[n];
            full.entry(n.to_ascii_lowercase()).or_insert(loc);
            if let Some((_, b)) = n.rsplit_once('/') {
                base.entry(b.to_ascii_lowercase()).or_insert(loc);
            }
        }
        for (k, v) in base {
            full.entry(k).or_insert(v);
        }
        EntryIndex {
            exact,
            folded: full,
        }
    }

    pub fn get(&self, name: &str) -> Option<ZipEntryLoc> {
        self.exact
            .get(name)
            .or_else(|| self.folded.get(&name.to_ascii_lowercase()))
            .copied()
    }
}

/// Reads `r` to the end, failing once more than `limit` bytes arrive (zip bombs: the zip crate
/// does not enforce an entry's declared size). `size_hint` only sizes the initial buffer.
pub fn read_limited<R: Read>(r: R, limit: u64, size_hint: u64) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size_hint.min(limit).min(64 << 20) as usize);
    r.take(limit.saturating_add(1)).read_to_end(&mut out)?;
    if out.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("zip entry larger than {limit} bytes"),
        ));
    }
    Ok(out)
}

/// Offset of the entry data given its local header offset (reads the 30-byte local header).
pub fn local_data_start<R: Read + Seek>(r: &mut R, header_offset: u64) -> io::Result<u64> {
    let mut h = [0u8; 30];
    r.seek(SeekFrom::Start(header_offset))?;
    r.read_exact(&mut h)?;
    if u32le(&h, 0) != 0x0403_4b50 {
        return Err(bad("bad local file header signature"));
    }
    Ok(header_offset + 30 + u16le(&h, 26) as u64 + u16le(&h, 28) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    #[test]
    fn matches_zip_crate() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.zip");
        {
            let mut w = zip::ZipWriter::new(File::create(&p).unwrap());
            for i in 0..50 {
                let opts = if i % 2 == 0 {
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated)
                } else {
                    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)
                };
                w.start_file(format!("{i}.fb2"), opts).unwrap();
                w.write_all(format!("<book>{}</book>", "x".repeat(i * 10)).as_bytes())
                    .unwrap();
            }
            w.finish().unwrap();
        }
        let cd = read_central_directory(&p).unwrap();
        assert_eq!(cd.len(), 50);
        let mut za = zip::ZipArchive::new(File::open(&p).unwrap()).unwrap();
        for i in 0..50 {
            let f = za.by_index(i).unwrap();
            let loc = cd[f.name()];
            assert_eq!(loc.offset, f.header_start());
            assert_eq!(loc.csize, f.compressed_size());
            assert_eq!(loc.usize, f.size());
            assert_eq!(loc.method, if i % 2 == 0 { 8 } else { 0 });
            let ds = f.data_start();
            drop(f);
            let mut file = File::open(&p).unwrap();
            let start = local_data_start(&mut file, loc.offset).unwrap();
            if let Some(ds) = ds {
                assert_eq!(start, ds);
            }
        }
    }

    #[test]
    fn lookup_rules() {
        let loc = |o| ZipEntryLoc {
            offset: o,
            csize: 1,
            usize: 1,
            method: 0,
        };
        let mut m = HashMap::new();
        m.insert("A.fb2".to_string(), loc(1));
        m.insert("dir/B.FB2".to_string(), loc(2));
        m.insert("a.fb2".to_string(), loc(3));
        let ix = EntryIndex::new(m);
        assert_eq!(ix.get("a.fb2").unwrap().offset, 3);
        assert_eq!(ix.get("A.FB2").unwrap().offset, 1);
        assert_eq!(ix.get("b.fb2").unwrap().offset, 2);
        assert!(ix.get("c.fb2").is_none());
        assert!(entry_matches("dir/B.FB2", "b.fb2"));
        assert!(!entry_matches("dir/B.FB2", "dir"));
    }

    #[test]
    fn not_a_zip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.zip");
        std::fs::write(&p, b"definitely not a zip file at all").unwrap();
        assert!(read_central_directory(&p).is_err());
    }
}
