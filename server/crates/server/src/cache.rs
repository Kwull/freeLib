//! Disk cache housekeeping: size-bounded LRU eviction of `cache/{out,covers,info}` (by
//! modification time, which cache hits refresh) and removal of stale temporary files.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use crate::state::AppState;

/// Cache kinds that are bounded by `FREELIB_CACHE_MAX_MB`.
pub const EVICTED: [&str; 3] = ["out", "covers", "info"];

/// Evict down to this share of the limit, so that eviction does not run after every write.
const LOW_WATER_PERCENT: u64 = 90;

/// Called after a cache file of `bytes` was written: schedules an eviction pass once enough new
/// data (5% of the limit) arrived since the last one.
pub fn written(st: &AppState, bytes: u64) {
    let max = st.cfg.cache_max_bytes;
    if max == 0 {
        return;
    }
    let total = st.cache_written.fetch_add(bytes, Ordering::Relaxed) + bytes;
    if total >= max / 20 && !st.cache_evicting.swap(true, Ordering::AcqRel) {
        st.cache_written.store(0, Ordering::Relaxed);
        let st = st.clone();
        tokio::task::spawn_blocking(move || {
            evict(&st.cfg.cache_dir, st.cfg.cache_max_bytes);
            st.cache_evicting.store(false, Ordering::Release);
        });
    }
}

struct Item {
    path: PathBuf,
    size: u64,
    mtime: SystemTime,
}

fn walk(dir: &Path, out: &mut Vec<Item>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        if md.is_dir() {
            walk(&e.path(), out);
        } else if md.is_file() {
            out.push(Item {
                path: e.path(),
                size: md.len(),
                mtime: md.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
}

/// Deletes least recently used files of the bounded cache kinds until their total size is at
/// most 90% of `max_bytes` (no-op when under the limit or `max_bytes` is 0). Returns the number
/// of bytes removed. Blocking.
pub fn evict(cache_dir: &Path, max_bytes: u64) -> u64 {
    if max_bytes == 0 {
        return 0;
    }
    let mut items = Vec::new();
    for k in EVICTED {
        walk(&cache_dir.join(k), &mut items);
    }
    let mut total: u64 = items.iter().map(|i| i.size).sum();
    if total <= max_bytes {
        return 0;
    }
    let target = max_bytes / 100 * LOW_WATER_PERCENT;
    items.sort_by_key(|i| i.mtime);
    let mut removed = 0;
    for i in items {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&i.path).is_ok() {
            total -= i.size;
            removed += i.size;
        }
    }
    if removed > 0 {
        tracing::info!(
            "cache eviction: removed {} MiB, {} MiB left",
            removed >> 20,
            total >> 20
        );
    }
    removed
}

/// `<name>.tmp<16 hex digits>`: an interrupted atomic write.
fn is_tmp_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .and_then(|e| e.strip_prefix(crate::output::TMP_EXT_PREFIX))
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn remove_tmp(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            remove_tmp(&p);
        } else if is_tmp_file(&p) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// At startup (no writes in flight): removes leftovers of interrupted atomic writes and the
/// Calibre scratch folder. Blocking.
pub fn clean_on_start(cache_dir: &Path) {
    for k in EVICTED {
        remove_tmp(&cache_dir.join(k));
    }
    let _ = std::fs::remove_dir_all(cache_dir.join("tmp"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn evicts_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out/1");
        std::fs::create_dir_all(&out).unwrap();
        let now = SystemTime::now();
        for i in 0..10u64 {
            let p = out.join(format!("{i}.epub"));
            std::fs::write(&p, vec![0u8; 1000]).unwrap();
            let f = std::fs::File::options().append(true).open(&p).unwrap();
            f.set_modified(now - Duration::from_secs(1000 - i * 10))
                .unwrap();
        }
        assert_eq!(evict(dir.path(), 20_000), 0);
        let removed = evict(dir.path(), 5_000);
        assert_eq!(removed, 6000, "down to 90% of 5000");
        for i in 0..6 {
            assert!(!out.join(format!("{i}.epub")).exists());
        }
        for i in 6..10 {
            assert!(out.join(format!("{i}.epub")).exists());
        }
    }

    #[test]
    fn temp_files() {
        assert!(is_tmp_file(Path::new("a/abc.tmp0123456789abcdef")));
        assert!(!is_tmp_file(Path::new("a/abc.epub")));
        assert!(!is_tmp_file(Path::new("a/abc.tmp01")));
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("covers/1");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("x.tmp0123456789abcdef"), b"x").unwrap();
        std::fs::write(d.join("x-full.jpg"), b"x").unwrap();
        clean_on_start(dir.path());
        assert!(!d.join("x.tmp0123456789abcdef").exists());
        assert!(d.join("x-full.jpg").exists());
    }
}
