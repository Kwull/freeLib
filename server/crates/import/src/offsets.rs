//! Resolve zip local-header offsets for an existing catalog (post-import step).
//!
//! The importer already does this while parsing when `ImportOptions::resolve_offsets` is set;
//! this standalone pass is for catalogs imported before the archives were available.
//! It writes to the catalog, so prefer running it on a catalog that is not being served
//! (readers may briefly see `SQLITE_BUSY` while the update commits).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::ImportError;
use crate::builder::Progress;
use crate::zipdir::read_central_directory;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OffsetStats {
    pub archives: u64,
    pub books: u64,
    pub resolved: u64,
    /// Books whose entry was not found in an existing archive.
    pub not_in_archive: u64,
    pub missing_archives: Vec<String>,
}

/// Read each archive's central directory once and store `arch_offset/arch_csize/arch_method`
/// for every zipped book of the catalog at `db_path`. Missing archives are skipped.
pub fn resolve_offsets(
    db_path: &Path,
    library_dir: &Path,
    progress: Progress,
    cancel: &AtomicBool,
) -> Result<OffsetStats, ImportError> {
    let mut conn = Connection::open(db_path)?;
    conn.busy_timeout(std::time::Duration::from_secs(30))?;
    let mut by_archive: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    {
        let mut st = conn.prepare("SELECT id, archive, file, ext FROM book WHERE archive <> ''")?;
        let mut q = st.query([])?;
        while let Some(r) = q.next()? {
            let file: String = r.get(2)?;
            let ext: String = r.get(3)?;
            let entry = if ext.is_empty() {
                file
            } else {
                format!("{file}.{ext}")
            };
            by_archive
                .entry(r.get(1)?)
                .or_default()
                .push((r.get(0)?, entry));
        }
    }
    let mut archives: Vec<String> = by_archive.keys().cloned().collect();
    archives.sort();
    let mut stats = OffsetStats {
        archives: archives.len() as u64,
        ..Default::default()
    };
    let total = archives.len() as u64;
    let tx = conn.transaction()?;
    {
        let mut upd = tx
            .prepare("UPDATE book SET arch_offset=?2, arch_csize=?3, arch_method=?4 WHERE id=?1")?;
        for (i, arch) in archives.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(ImportError::Cancelled);
            }
            let books = &by_archive[arch];
            stats.books += books.len() as u64;
            match read_central_directory(&library_dir.join(arch)) {
                Ok(cd) => {
                    for (id, entry) in books {
                        match cd.get(entry) {
                            Some(l) => {
                                upd.execute(params![
                                    id,
                                    l.offset as i64,
                                    l.csize as i64,
                                    l.method as i64
                                ])?;
                                stats.resolved += 1;
                            }
                            None => stats.not_in_archive += 1,
                        }
                    }
                }
                Err(_) => stats.missing_archives.push(arch.clone()),
            }
            progress(i as u64 + 1, total, arch);
        }
    }
    tx.commit()?;
    Ok(stats)
}
