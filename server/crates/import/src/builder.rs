//! Catalog builder: INPX → `lib_<id>.new.db` → atomic rename to `lib_<id>.db`.
//!
//! `.inp` parts are decompressed and parsed in parallel (rayon, one zip handle per worker);
//! a single writer thread inserts rows in part order, so book ids are deterministic for a
//! given INPX. Authors, series, counts and the letter index are aggregated in memory and
//! written at the end, followed by indexes, FTS optimisation, `ANALYZE` and `meta`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::time::Instant;

use rayon::prelude::*;
use rusqlite::{params, Connection};
use serde::Serialize;
use zip::ZipArchive;

use freelib_catalog::genres::genres;
use freelib_catalog::normalize::{letter_of, normalize};
use freelib_catalog::schema::{create_catalog_indexes, create_catalog_tables, CATALOG_SCHEMA_VERSION};
use freelib_catalog::util::{now_millis, now_rfc3339};

use crate::inpx::{self, InpxInfo, ParseOptions, RawAuthor, RawBook};
use crate::zipdir::{read_central_directory, ZipEntryLoc};
use crate::ImportError;

/// Progress callback: `(done, total, message)`. Parts count as one step each, followed by
/// [`FINISH_STEPS`] finishing steps.
pub type Progress<'a> = &'a (dyn Fn(u64, u64, &str) + Sync);

/// Number of steps reported after all parts are written.
pub const FINISH_STEPS: u64 = 5;

/// What to import and how.
#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// The `.inpx` file.
    pub inpx: PathBuf,
    /// Final catalog path, e.g. `/data/lib_3.db`; the builder writes `lib_3.new.db` next to it.
    pub db_path: PathBuf,
    /// Library folder (where the zip archives live). Needed for `resolve_offsets`.
    pub library_dir: Option<PathBuf>,
    pub first_author_only: bool,
    pub skip_deleted: bool,
    /// Read each archive's central directory and store `arch_offset/csize/method`.
    /// Missing archives are skipped (offsets stay NULL).
    pub resolve_offsets: bool,
    /// Parser threads; 0 = CPU cores − 1 (the writer runs on its own thread).
    pub threads: usize,
}

/// Summary of an import.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStats {
    pub parts: u64,
    pub books: u64,
    pub live_books: u64,
    pub authors: u64,
    pub series: u64,
    /// Records whose LIBID was already used: stored under their `file:` key instead.
    pub duplicate_lib_ids: u64,
    /// Records dropped because both keys were already taken (exact duplicates).
    pub dropped_duplicates: u64,
    pub offsets_resolved: u64,
    /// Archives referenced by the INPX but not found / unreadable in the library folder.
    pub missing_archives: Vec<String>,
    pub catalog_version: i64,
    pub inpx_version: Option<String>,
    pub elapsed_ms: u64,
    /// `(phase, milliseconds)`.
    pub timings: Vec<(String, u64)>,
}

/// `lib_3.db` → `lib_3.new.db`.
pub fn new_db_path(db_path: &Path) -> PathBuf {
    let stem = db_path.file_stem().and_then(|s| s.to_str()).unwrap_or("catalog");
    db_path.with_file_name(format!("{stem}.new.db"))
}

struct PartOut {
    idx: usize,
    name: String,
    books: Vec<RawBook>,
    missing: Vec<String>,
}

fn process_part(
    za: &mut ZipArchive<File>,
    idx: usize,
    name: &str,
    info: &InpxInfo,
    popts: ParseOptions,
    lib_dir: Option<&Path>,
) -> Result<PartOut, ImportError> {
    let data = inpx::read_part(za, name)?;
    let mut books = inpx::parse_inp(&data, name, &info.structure, popts);
    let mut missing = Vec::new();
    if let Some(dir) = lib_dir {
        let mut dirs: HashMap<String, Option<HashMap<String, ZipEntryLoc>>> = HashMap::new();
        for b in books.iter_mut().filter(|b| !b.archive.is_empty()) {
            let cd = dirs.entry(b.archive.clone()).or_insert_with(|| {
                let r = read_central_directory(&dir.join(&b.archive)).ok();
                if r.is_none() {
                    missing.push(b.archive.clone());
                }
                r
            });
            if let Some(cd) = cd {
                b.loc = cd.get(&b.entry_name()).copied();
            }
        }
    }
    Ok(PartOut { idx, name: name.to_string(), books, missing })
}

struct AuthorAgg {
    a: RawAuthor,
    name: String,
    sort_key: String,
    live: i64,
}

struct SeriesAgg {
    name: String,
    sort_key: String,
    live: i64,
    /// author id → books in this series
    authors: HashMap<i64, u32>,
}

/// In-memory aggregation state of the writer.
#[derive(Default)]
struct Agg {
    authors: HashMap<String, i64>,
    author_rows: Vec<AuthorAgg>,
    series: HashMap<String, i64>,
    series_rows: Vec<SeriesAgg>,
    genre_counts: HashMap<u16, i64>,
    lang_counts: HashMap<String, i64>,
    keys: HashSet<String>,
    next_book_id: i64,
}

impl Agg {
    fn author_id(&mut self, a: &RawAuthor) -> i64 {
        let name = a.display();
        let key = normalize(&name);
        if let Some(&id) = self.authors.get(&key) {
            return id;
        }
        self.author_rows.push(AuthorAgg { a: a.clone(), name, sort_key: key.clone(), live: 0 });
        let id = self.author_rows.len() as i64;
        self.authors.insert(key, id);
        id
    }

    fn series_id(&mut self, name: &str) -> Option<i64> {
        if name.is_empty() {
            return None;
        }
        let key = normalize(name);
        if let Some(&id) = self.series.get(&key) {
            return Some(id);
        }
        self.series_rows.push(SeriesAgg { name: name.to_string(), sort_key: key.clone(), live: 0, authors: HashMap::new() });
        let id = self.series_rows.len() as i64;
        self.series.insert(key, id);
        Some(id)
    }
}

/// Build the catalog for `opts` and swap it in. On error or cancellation the previous
/// catalog is untouched and the partial `.new.db` is removed.
pub fn import_inpx(opts: &ImportOptions, progress: Progress, cancel: &AtomicBool) -> Result<ImportStats, ImportError> {
    let new_path = new_db_path(&opts.db_path);
    let r = build(opts, &new_path, progress, cancel);
    if r.is_err() {
        let _ = std::fs::remove_file(&new_path);
    }
    r
}

fn previous_version(db_path: &Path) -> i64 {
    freelib_catalog::open_read_only(db_path)
        .and_then(|c| c.query_row("SELECT value FROM meta WHERE key='catalog_version'", [], |r| r.get::<_, String>(0)))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn build(opts: &ImportOptions, new_path: &Path, progress: Progress, cancel: &AtomicBool) -> Result<ImportStats, ImportError> {
    let started = Instant::now();
    let mut stats = ImportStats::default();
    let mut phase = Instant::now();
    let mut mark = |stats: &mut ImportStats, name: &str| {
        stats.timings.push((name.to_string(), phase.elapsed().as_millis() as u64));
        phase = Instant::now();
    };

    let info = inpx::read_info(&opts.inpx)?;
    let total = info.parts.len() as u64 + FINISH_STEPS;
    stats.parts = info.parts.len() as u64;
    stats.inpx_version = info.version.clone();
    progress(0, total, &format!("Reading {} parts", info.parts.len()));

    let _ = std::fs::remove_file(new_path);
    let mut conn = Connection::open(new_path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA locking_mode=EXCLUSIVE; \
         PRAGMA cache_size=-262144; PRAGMA temp_store=MEMORY;",
    )?;
    create_catalog_tables(&conn)?;

    let popts = ParseOptions { skip_deleted: opts.skip_deleted, first_author_only: opts.first_author_only };
    let lib_dir = if opts.resolve_offsets { opts.library_dir.as_deref() } else { None };
    let threads = if opts.threads > 0 {
        opts.threads
    } else {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).saturating_sub(1).max(1)
    };
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().map_err(|e| ImportError::Other(e.to_string()))?;

    let mut agg = Agg { next_book_id: 1, ..Default::default() };
    let tx_conn = conn.transaction()?;
    let write_result: Result<(), ImportError> = std::thread::scope(|s| {
        let (tx, rx) = sync_channel::<Result<PartOut, ImportError>>(threads * 2);
        let info_ref = &info;
        let inpx_path = &opts.inpx;
        s.spawn(move || {
            pool.install(|| {
                info_ref.parts.par_iter().enumerate().for_each_init(
                    || {
                        let za = File::open(inpx_path)
                            .map_err(ImportError::from)
                            .and_then(|f| ZipArchive::new(f).map_err(ImportError::from));
                        (tx.clone(), za)
                    },
                    |(tx, za), (idx, name)| {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        let r = match za {
                            Ok(za) => process_part(za, idx, name, info_ref, popts, lib_dir),
                            Err(e) => Err(ImportError::Other(format!("cannot open INPX: {e}"))),
                        };
                        let _ = tx.send(r);
                    },
                );
            });
            drop(tx);
        });

        let mut w = Writer::new(&tx_conn)?;
        let mut pending: BTreeMap<usize, PartOut> = BTreeMap::new();
        let mut next = 0usize;
        let mut done = 0u64;
        for r in rx.iter() {
            if cancel.load(Ordering::Relaxed) {
                return Err(ImportError::Cancelled);
            }
            let part = r?;
            pending.insert(part.idx, part);
            while let Some(part) = pending.remove(&next) {
                for b in &part.books {
                    w.add(&mut agg, b, &mut stats)?;
                }
                stats.missing_archives.extend(part.missing);
                next += 1;
                done += 1;
                progress(done, total, &format!("{} ({} books)", part.name, stats.books));
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(ImportError::Cancelled);
        }
        if next != info.parts.len() {
            return Err(ImportError::Other("parser stopped early".into()));
        }
        Ok(())
    });
    write_result?;
    mark(&mut stats, "parse+insert books");
    stats.missing_archives.sort();
    stats.missing_archives.dedup();

    let parts = info.parts.len() as u64;
    progress(parts, total, "Writing authors, series and counts");
    write_aggregates(&tx_conn, &agg)?;
    stats.authors = agg.author_rows.len() as u64;
    stats.series = agg.series_rows.len() as u64;
    mark(&mut stats, "authors/series/counts");
    check_cancel(cancel)?;

    progress(parts + 1, total, "Building indexes");
    create_catalog_indexes(&tx_conn)?;
    mark(&mut stats, "indexes");
    check_cancel(cancel)?;

    progress(parts + 2, total, "Optimizing full-text index");
    tx_conn.execute_batch(
        "INSERT INTO book_fts(book_fts) VALUES('optimize'); \
         INSERT INTO author_fts(author_fts) VALUES('optimize'); \
         INSERT INTO series_fts(series_fts) VALUES('optimize');",
    )?;
    mark(&mut stats, "fts optimize");
    check_cancel(cancel)?;

    progress(parts + 3, total, "Analyzing");
    tx_conn.execute_batch("ANALYZE")?;
    mark(&mut stats, "analyze");

    stats.catalog_version = now_millis().max(previous_version(&opts.db_path) + 1);
    let meta: Vec<(&str, String)> = vec![
        ("schema_version", CATALOG_SCHEMA_VERSION.to_string()),
        ("inpx_version", info.version.clone().unwrap_or_default()),
        ("collection_name", info.collection_name.clone().unwrap_or_default()),
        ("imported_at", now_rfc3339()),
        ("catalog_version", stats.catalog_version.to_string()),
        ("source_inpx", opts.inpx.to_string_lossy().into_owned()),
        ("first_author_only", (opts.first_author_only as i32).to_string()),
        ("skip_deleted", (opts.skip_deleted as i32).to_string()),
        ("book_count", stats.books.to_string()),
        ("live_book_count", stats.live_books.to_string()),
        ("author_count", stats.authors.to_string()),
        ("series_count", stats.series.to_string()),
    ];
    {
        let mut st = tx_conn.prepare("INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)")?;
        for (k, v) in &meta {
            st.execute(params![k, v])?;
        }
    }
    tx_conn.commit()?;
    // Read-only afterwards: a rollback journal needs no -wal/-shm files next to the catalog.
    conn.pragma_update(None, "journal_mode", "DELETE")?;
    conn.pragma_update(None, "locking_mode", "NORMAL")?;
    drop(conn);
    File::open(new_path)?.sync_all()?;
    check_cancel(cancel)?;

    progress(parts + 4, total, "Activating new catalog");
    std::fs::rename(new_path, &opts.db_path)?;
    if let Some(d) = opts.db_path.parent().and_then(|dir| File::open(dir).ok()) {
        let _ = d.sync_all();
    }
    mark(&mut stats, "finish");
    stats.elapsed_ms = started.elapsed().as_millis() as u64;
    progress(total, total, &format!("Imported {} books", stats.books));
    Ok(stats)
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), ImportError> {
    if cancel.load(Ordering::Relaxed) { Err(ImportError::Cancelled) } else { Ok(()) }
}

/// Prepared insert statements of the single writer.
struct Writer<'c> {
    book: rusqlite::Statement<'c>,
    ba: rusqlite::Statement<'c>,
    bg: rusqlite::Statement<'c>,
    fts: rusqlite::Statement<'c>,
    gbuf: Vec<u16>,
}

impl<'c> Writer<'c> {
    fn new(conn: &'c Connection) -> rusqlite::Result<Self> {
        Ok(Writer {
            book: conn.prepare(
                "INSERT INTO book(id, book_key, title, sort_key, series_id, serno, first_author_id, lang, ext, file, \
                 archive, folder, size, date, deleted, lib_id, stars, keywords, arch_offset, arch_csize, arch_method) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
            )?,
            ba: conn.prepare("INSERT OR IGNORE INTO book_author(book_id, author_id, pos) VALUES (?1,?2,?3)")?,
            bg: conn.prepare("INSERT OR IGNORE INTO book_genre(book_id, genre_id) VALUES (?1,?2)")?,
            fts: conn.prepare("INSERT INTO book_fts(rowid, title, authors, series, keywords) VALUES (?1,?2,?3,?4,?5)")?,
            gbuf: Vec::with_capacity(8),
        })
    }

    fn add(&mut self, agg: &mut Agg, b: &RawBook, stats: &mut ImportStats) -> Result<(), ImportError> {
        let mut key = b.book_key();
        if agg.keys.contains(&key) {
            if b.lib_id.is_none() {
                stats.dropped_duplicates += 1;
                return Ok(());
            }
            key = b.file_key();
            if agg.keys.contains(&key) {
                stats.dropped_duplicates += 1;
                return Ok(());
            }
            stats.duplicate_lib_ids += 1;
        }
        let id = agg.next_book_id;
        agg.next_book_id += 1;
        let live = !b.deleted;

        let author_ids: Vec<i64> = b.authors.iter().map(|a| agg.author_id(a)).collect();
        let series_id = agg.series_id(&b.series);
        let series_name = series_id.map(|s| agg.series_rows[(s - 1) as usize].name.clone()).unwrap_or_default();

        self.book.execute(params![
            id,
            key,
            b.title,
            normalize(&b.title),
            series_id,
            series_id.and(b.serno),
            author_ids[0],
            b.lang,
            b.ext,
            b.file,
            b.archive,
            b.folder,
            b.size,
            b.date,
            b.deleted as i32,
            b.lib_id,
            b.stars,
            b.keywords,
            b.loc.map(|l| l.offset as i64),
            b.loc.map(|l| l.csize as i64),
            b.loc.map(|l| l.method as i64),
        ])?;
        if b.loc.is_some() {
            stats.offsets_resolved += 1;
        }
        for (pos, aid) in author_ids.iter().enumerate() {
            self.ba.execute(params![id, aid, pos as i64])?;
        }

        // Genres: map codes to ids, dedupe.
        self.gbuf.clear();
        let g = genres();
        for code in &b.genres {
            if let Some(gid) = g.resolve(code).filter(|gid| !self.gbuf.contains(gid)) {
                self.gbuf.push(gid);
            }
        }
        for gid in &self.gbuf {
            self.bg.execute(params![id, gid])?;
        }

        let authors_text: Vec<String> = b.authors.iter().map(|a| normalize(&a.display())).collect();
        self.fts.execute(params![
            id,
            normalize(&b.title),
            authors_text.join(" "),
            normalize(&series_name),
            normalize(&b.keywords)
        ])?;

        agg.keys.insert(key);
        stats.books += 1;
        if live {
            stats.live_books += 1;
            for aid in &author_ids {
                agg.author_rows[(*aid - 1) as usize].live += 1;
            }
            if let Some(sid) = series_id {
                let s = &mut agg.series_rows[(sid - 1) as usize];
                s.live += 1;
                *s.authors.entry(author_ids[0]).or_default() += 1;
            }
            let mut tops: Vec<u16> = Vec::with_capacity(2);
            for &gid in &self.gbuf {
                let top = g.top(gid);
                if gid != top {
                    *agg.genre_counts.entry(gid).or_default() += 1;
                }
                if !tops.contains(&top) {
                    tops.push(top);
                    *agg.genre_counts.entry(top).or_default() += 1;
                }
            }
            *agg.lang_counts.entry(b.lang.clone()).or_default() += 1;
        } else if let Some(sid) = series_id {
            // Deleted books still count for the "main authors" of a series without live books.
            agg.series_rows[(sid - 1) as usize].authors.entry(author_ids[0]).or_default();
        }
        Ok(())
    }
}

fn write_letter_index(conn: &Connection, kind: &str, keys: &mut [(String, i64)]) -> rusqlite::Result<()> {
    keys.sort();
    let mut st = conn.prepare("INSERT INTO letter_index(kind, letter, count, first_pos) VALUES (?1,?2,?3,?4)")?;
    let mut letters: Vec<(String, i64, i64)> = Vec::new();
    let mut pos_of: HashMap<String, usize> = HashMap::new();
    for (pos, (k, _)) in keys.iter().enumerate() {
        let l = letter_of(k);
        match pos_of.get(&l) {
            Some(&i) => letters[i].1 += 1,
            None => {
                pos_of.insert(l.clone(), letters.len());
                letters.push((l, 1, pos as i64));
            }
        }
    }
    for (l, c, p) in letters {
        st.execute(params![kind, l, c, p])?;
    }
    Ok(())
}

fn write_aggregates(conn: &Connection, agg: &Agg) -> rusqlite::Result<()> {
    {
        let mut st = conn.prepare(
            "INSERT INTO author(id, last, first, middle, name, sort_key, book_count) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        )?;
        let mut fts = conn.prepare("INSERT INTO author_fts(rowid, name) VALUES (?1, ?2)")?;
        for (i, a) in agg.author_rows.iter().enumerate() {
            let id = i as i64 + 1;
            st.execute(params![id, a.a.last, a.a.first, a.a.middle, a.name, a.sort_key, a.live])?;
            fts.execute(params![id, a.sort_key])?;
        }
    }
    {
        let mut st =
            conn.prepare("INSERT INTO series(id, name, sort_key, book_count, authors) VALUES (?1,?2,?3,?4,?5)")?;
        let mut fts = conn.prepare("INSERT INTO series_fts(rowid, name) VALUES (?1, ?2)")?;
        for (i, s) in agg.series_rows.iter().enumerate() {
            let id = i as i64 + 1;
            let mut top: Vec<(&i64, &u32)> = s.authors.iter().collect();
            top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let names: Vec<&str> = top.iter().take(2).map(|(aid, _)| agg.author_rows[(**aid - 1) as usize].name.as_str()).collect();
            st.execute(params![id, s.name, s.sort_key, s.live, names.join(", ")])?;
            fts.execute(params![id, s.sort_key])?;
        }
    }
    {
        let mut st = conn.prepare("INSERT INTO genre_count(genre_id, count) VALUES (?1, ?2)")?;
        for (g, c) in &agg.genre_counts {
            st.execute(params![g, c])?;
        }
        let mut st = conn.prepare("INSERT INTO lang_count(lang, count) VALUES (?1, ?2)")?;
        for (l, c) in &agg.lang_counts {
            st.execute(params![l, c])?;
        }
    }
    let mut keys: Vec<(String, i64)> =
        agg.author_rows.iter().enumerate().map(|(i, a)| (a.sort_key.clone(), i as i64 + 1)).collect();
    write_letter_index(conn, "author", &mut keys)?;
    let mut keys: Vec<(String, i64)> =
        agg.series_rows.iter().enumerate().map(|(i, s)| (s.sort_key.clone(), i as i64 + 1)).collect();
    write_letter_index(conn, "series", &mut keys)?;
    Ok(())
}
