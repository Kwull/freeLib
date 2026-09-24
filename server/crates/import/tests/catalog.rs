//! End-to-end: generate an INPX (+ archives), import it, and exercise every catalog query.

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use freelib_catalog::{
    genres, normalize, BookFilter, BookSelector, Catalog, CatalogHandle, Page, SearchKind, SearchQuery,
};
use freelib_import::synth::{generate, GenOptions};
use freelib_import::{import_inpx, resolve_offsets, ImportError, ImportOptions};
use zip::write::SimpleFileOptions;

fn no_progress(_: u64, _: u64, _: &str) {}

fn import(inpx: &Path, db: &Path, lib: Option<&Path>) -> freelib_import::ImportStats {
    let opts = ImportOptions {
        inpx: inpx.to_path_buf(),
        db_path: db.to_path_buf(),
        library_dir: lib.map(Path::to_path_buf),
        resolve_offsets: lib.is_some(),
        ..Default::default()
    };
    import_inpx(&opts, &no_progress, &AtomicBool::new(false)).unwrap()
}

fn all_pages(cat: &Catalog, sel: &BookSelector, f: &BookFilter, limit: usize) -> Vec<freelib_catalog::Book> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let p = cat.books(sel, f, &Page { cursor: cursor.clone(), limit }).unwrap();
        out.extend(p.books);
        match p.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    out
}

#[test]
fn generated_catalog_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let inpx = dir.path().join("lib.inpx");
    let lib = dir.path().join("lib");
    let gs = generate(&inpx, &GenOptions { books: 3000, per_archive: 500, seed: 11, files_dir: Some(lib.clone()), structure_info: true })
        .unwrap();
    assert_eq!(gs.parts, 6);
    let db = dir.path().join("lib_1.db");

    let mut progress_calls = std::sync::Mutex::new(Vec::new());
    let opts = ImportOptions {
        inpx: inpx.clone(),
        db_path: db.clone(),
        library_dir: Some(lib.clone()),
        resolve_offsets: true,
        ..Default::default()
    };
    let stats = import_inpx(
        &opts,
        &|d, t, m: &str| progress_calls.lock().unwrap().push((d, t, m.to_string())),
        &AtomicBool::new(false),
    )
    .unwrap();
    let calls = progress_calls.get_mut().unwrap();
    assert_eq!(calls.last().unwrap().0, calls.last().unwrap().1);
    assert!(calls.windows(2).all(|w| w[0].0 <= w[1].0));
    assert_eq!(stats.books, 3000);
    assert_eq!(stats.offsets_resolved, 3000);
    assert!(stats.missing_archives.is_empty());
    assert!(!new_db_exists(&db));

    let handle = CatalogHandle::new(&db);
    let cat = handle.get().expect("catalog opened");
    let st = cat.stats().clone();
    assert_eq!(st.book_count, 3000);
    assert_eq!(st.live_book_count as u64, stats.live_books);
    assert!(st.author_count > 100 && st.series_count > 10);
    assert_eq!(st.inpx_version.as_deref(), Some("20260901"));
    assert!(st.collection_name.as_deref().unwrap().starts_with("Синтетическая"));
    assert!(st.imported_at.is_some() && st.catalog_version > 0);

    // --- authors / series lists
    for (list, count) in [(cat.authors().unwrap(), st.author_count), (cat.series_list().unwrap(), st.series_count)] {
        assert_eq!(list.rows.len() as i64, count);
        let keys: Vec<String> = list.rows.iter().map(|r| normalize(&r.1)).collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "sorted by sort key");
        assert_eq!(list.letters.iter().map(|l| l.1).sum::<i64>(), count);
        for (letter, _, first) in &list.letters {
            assert_eq!(&freelib_catalog::letter_of(&keys[*first as usize]), letter);
        }
    }
    let authors = cat.authors().unwrap();
    assert!(authors.rows.iter().map(|r| r.2).max().unwrap() > 5);

    // --- genres / languages
    let g = cat.genres().unwrap();
    assert_eq!(g.len(), 322);
    for top in g.iter().filter(|x| x.parent == 0) {
        let kids: Vec<i64> = g.iter().filter(|x| x.parent == top.id).map(|x| x.count).collect();
        // distinct books of the group: at least its largest child, at most the sum (+ direct books for "Прочее")
        assert!(top.count >= kids.iter().copied().max().unwrap_or(0), "group {}", top.id);
        if top.id != freelib_catalog::GENRE_OTHER {
            assert!(top.count <= kids.iter().sum::<i64>(), "group {}", top.id);
        }
    }
    assert!(g.iter().any(|x| x.id == 118 && x.count > 0), "unknown sf code → Фантастика: прочее");
    let langs = cat.languages().unwrap();
    assert_eq!(langs[0].0, "ru");
    assert_eq!(langs.iter().map(|l| l.1).sum::<i64>(), st.live_book_count);

    // --- books by author, pagination and filters
    let top = authors.rows.iter().max_by_key(|r| r.2).unwrap();
    let all = all_pages(&cat, &BookSelector::Author(top.0), &BookFilter::default(), 7);
    assert_eq!(all.len() as i64, top.2);
    let first = cat.books(&BookSelector::Author(top.0), &BookFilter::default(), &Page { cursor: None, limit: 7 }).unwrap();
    assert_eq!(first.total, top.2);
    assert_eq!(all.iter().map(|b| b.id).collect::<HashSet<_>>().len(), all.len(), "no duplicates across pages");
    assert!(all.iter().all(|b| b.authors.iter().any(|a| a.id == top.0) && !b.deleted));
    // series books first, grouped and ordered by serno
    let series_keys: Vec<Option<String>> = all.iter().map(|b| b.series.as_ref().map(|s| normalize(&s.name))).collect();
    let first_none = series_keys.iter().position(Option::is_none).unwrap_or(series_keys.len());
    assert!(series_keys[first_none..].iter().all(Option::is_none));
    assert!(series_keys[..first_none].windows(2).all(|w| w[0] <= w[1]));
    let with_del = cat.books(&BookSelector::Author(top.0), &BookFilter { include_deleted: true, ..Default::default() }, &Page::default()).unwrap();
    assert!(with_del.total >= top.2);
    let ru = cat
        .books(&BookSelector::Author(top.0), &BookFilter { langs: vec!["ru".into()], ext: Some("FB2".into()), include_deleted: false }, &Page::default())
        .unwrap();
    assert!(ru.books.iter().all(|b| b.lang == "ru" && b.ext == "fb2"));
    assert!(ru.total <= top.2);
    assert!(cat.books(&BookSelector::Author(top.0), &BookFilter::default(), &Page { cursor: Some("x".into()), limit: 5 }).is_err());

    // --- books by series
    let series = cat.series_list().unwrap();
    let big_series = series.rows.iter().max_by_key(|r| r.2).unwrap();
    let sb = cat.books(&BookSelector::Series(big_series.0), &BookFilter::default(), &Page::default()).unwrap();
    assert_eq!(sb.total, big_series.2);
    let sernos: Vec<i64> = sb.books.iter().map(|b| b.serno.unwrap_or(i64::MAX)).collect();
    assert!(sernos.windows(2).all(|w| w[0] <= w[1]));
    let info = cat.series(big_series.0).unwrap().unwrap();
    assert!(!info.authors.is_empty());

    // --- books by genre: leaf and group, both strategies agree
    let leaf = g.iter().filter(|x| x.parent != 0).max_by_key(|x| x.count).unwrap();
    let lb = all_pages(&cat, &BookSelector::Genre(leaf.id), &BookFilter::default(), 50);
    assert_eq!(lb.len() as i64, leaf.count);
    assert!(lb.iter().all(|b| b.genres.contains(&leaf.id)));
    assert!(lb.windows(2).all(|w| w[0].date >= w[1].date));
    let group = g.iter().filter(|x| x.parent == 0).max_by_key(|x| x.count).unwrap();
    let small = cat.books(&BookSelector::Genre(group.id), &BookFilter::default(), &Page { cursor: Some("10".into()), limit: 40 }).unwrap();
    assert_eq!(small.total, group.count);
    let mut cat2 = Catalog::open(&db).unwrap();
    cat2.set_big_genre_threshold(1);
    let big = cat2.books(&BookSelector::Genre(group.id), &BookFilter::default(), &Page { cursor: Some("10".into()), limit: 40 }).unwrap();
    assert_eq!(big.total, small.total);
    assert_eq!(big.books, small.books);
    let f_ru = BookFilter { langs: vec!["ru".into()], ..Default::default() };
    let a = cat.books(&BookSelector::Genre(group.id), &f_ru, &Page { cursor: None, limit: 5 }).unwrap();
    let b = cat2.books(&BookSelector::Genre(group.id), &f_ru, &Page { cursor: None, limit: 5 }).unwrap();
    assert_eq!((a.total, &a.books), (b.total, &b.books));
    assert_eq!(a.total as usize, all_pages(&cat, &BookSelector::Genre(group.id), &f_ru, 500).len());
    let kids = genres().with_descendants(group.id);
    assert!(a.books.iter().all(|bk| bk.genres.iter().any(|x| kids.contains(x))));

    // --- since
    let since = "2020-01-01";
    let sp = cat.books(&BookSelector::Since(since.into()), &BookFilter::default(), &Page { cursor: None, limit: 100 }).unwrap();
    assert!(sp.books.iter().all(|b| b.date.as_str() >= since));
    assert!(sp.books.windows(2).all(|w| w[0].date >= w[1].date));
    assert_eq!(sp.total as usize, all_pages(&cat, &BookSelector::Since(since.into()), &BookFilter::default(), 1000).len());
    assert_eq!(cat.count_newer_than("2019-12-31").unwrap(), sp.total);

    // --- ids (shelves) and keys
    let ids: Vec<i64> = vec![5, 1, 2999, 77, 123_456];
    let ib = cat.books(&BookSelector::Ids(ids.clone()), &BookFilter { include_deleted: true, ..Default::default() }, &Page::default()).unwrap();
    assert_eq!(ib.total, 4);
    let by_ids = cat.books_by_ids(&ids).unwrap();
    assert_eq!(by_ids.iter().map(|b| b.id).collect::<Vec<_>>(), vec![5, 1, 2999, 77]);
    let keys = cat.keys_by_ids(&[1, 2, 3]).unwrap();
    assert_eq!(keys.len(), 3);
    let back = cat.ids_by_keys(&keys.iter().map(|k| k.1.clone()).chain(["lib:nope".to_string()]).collect::<Vec<_>>()).unwrap();
    assert_eq!(back.len(), 3);
    assert!(keys.iter().all(|(id, k)| back.contains(&(k.clone(), *id))));

    // --- book detail + offsets match the zip crate
    let d = cat.book(10).unwrap().unwrap();
    assert_eq!(d.book.id, 10);
    assert!(!d.book.authors.is_empty() && !d.book.title.is_empty());
    assert_eq!(d.book.key, format!("lib:{}", d.lib_id.unwrap()));
    assert!(d.archive.ends_with(".zip"));
    let mut za = zip::ZipArchive::new(File::open(lib.join(&d.archive)).unwrap()).unwrap();
    let entry = za.by_name(&d.entry_name()).unwrap();
    assert_eq!(d.arch_offset, Some(entry.header_start() as i64));
    assert_eq!(d.arch_csize, Some(entry.compressed_size() as i64));
    assert_eq!(d.arch_method, Some(8));
    assert_eq!(d.book.size as u64, entry.size());
    assert!(d.display_file().contains(" / "));
    assert!(cat.book(999_999).unwrap().is_none());
    assert!(cat.author(top.0).unwrap().is_some());

    // --- search
    let probe = cat.book(42).unwrap().unwrap().book;
    let word = probe.title.split(|c: char| !c.is_alphanumeric()).find(|w| w.chars().count() >= 3).unwrap().to_string();
    let r = cat
        .search(&SearchQuery { q: word.clone(), include_deleted: true, limit: 1000, ..Default::default() })
        .unwrap();
    assert!(r.total > 0);
    assert!(r.books.iter().any(|b| b.id == 42) || r.total > 1000, "{word}");
    assert_eq!(r.facets.lang.iter().map(|x| x.1).sum::<i64>(), r.total);
    assert_eq!(r.facets.ext.iter().map(|x| x.1).sum::<i64>(), r.total);
    // prefix + author word
    let last = &probe.authors[0].name.split(' ').next().unwrap().to_string();
    let prefix: String = last.chars().take(4).collect();
    let r = cat.search(&SearchQuery { q: prefix.clone(), limit: 50, ..Default::default() }).unwrap();
    assert!(!r.authors.is_empty(), "author prefix {prefix}");
    assert!(r.authors.iter().all(|a| normalize(&a.name).split(' ').any(|w| w.starts_with(&normalize(&prefix)))));
    let ra = cat.search(&SearchQuery { q: prefix.clone(), kind: SearchKind::Authors, ..Default::default() }).unwrap();
    assert!(ra.books.is_empty() && !ra.authors.is_empty());
    // filters narrow; facets ignore their own dimension
    let rf = cat
        .search(&SearchQuery { q: prefix.clone(), langs: vec!["en".into()], limit: 50, ..Default::default() })
        .unwrap();
    assert!(rf.books.iter().all(|b| b.lang == "en"));
    assert_eq!(rf.facets.lang, r.facets.lang);
    let rg = cat
        .search(&SearchQuery { q: prefix.clone(), genres: vec![group.id], limit: 50, ..Default::default() })
        .unwrap();
    assert!(rg.books.iter().all(|b| b.genres.iter().any(|x| kids.contains(x))));
    let rd = cat
        .search(&SearchQuery { q: prefix, from: Some("2015-01-01".into()), to: Some("2016-12-31".into()), limit: 50, ..Default::default() })
        .unwrap();
    assert!(rd.books.iter().all(|b| b.date.as_str() >= "2015-01-01" && b.date.as_str() <= "2016-12-31"));
    // series search
    let sname = &info.name;
    let rs = cat.search(&SearchQuery { q: sname.clone(), kind: SearchKind::Series, ..Default::default() }).unwrap();
    assert!(rs.series.iter().any(|s| s.id == info.id), "{sname}");
    // ё / е equivalence and too-short queries
    let yo = cat.search(&SearchQuery { q: "Тёмный".into(), limit: 5, ..Default::default() }).unwrap();
    let ye = cat.search(&SearchQuery { q: "темный".into(), limit: 5, ..Default::default() }).unwrap();
    assert_eq!(yo.total, ye.total);
    assert!(yo.total > 0);
    assert_eq!(cat.search(&SearchQuery { q: "а".into(), ..Default::default() }).unwrap().total, 0);

    // --- re-import and reload: version changes, old handle keeps working
    let v1 = cat.catalog_version();
    import(&inpx, &db, None);
    let cat_new = handle.reload().unwrap();
    assert!(cat_new.catalog_version() > v1);
    assert_eq!(cat.books_by_ids(&[1]).unwrap().len(), 1, "old catalog still readable");
    assert_eq!(cat_new.book(10).unwrap().unwrap().arch_offset, None, "no offsets without library dir");

    // --- standalone offset resolution
    drop(cat_new);
    handle.close();
    let os = resolve_offsets(&db, &lib, &no_progress, &AtomicBool::new(false)).unwrap();
    assert_eq!(os.resolved, 3000);
    assert_eq!(handle.reload().unwrap().book(10).unwrap().unwrap().arch_offset, d.arch_offset);

    // --- cancellation leaves the current catalog alone
    let cancel = AtomicBool::new(true);
    let before = handle.get().unwrap().catalog_version();
    let r = import_inpx(&ImportOptions { inpx: inpx.clone(), db_path: db.clone(), ..Default::default() }, &no_progress, &cancel);
    assert!(matches!(r, Err(ImportError::Cancelled)));
    assert!(!new_db_exists(&db));
    assert_eq!(Catalog::open(&db).unwrap().catalog_version(), before);
    cancel.store(false, Ordering::Relaxed);
}

fn new_db_exists(db: &Path) -> bool {
    freelib_import::new_db_path(db).exists()
}

fn write_inpx(path: &Path, structure: Option<&str>, parts: &[(&str, Vec<Vec<&str>>)]) {
    let mut z = zip::ZipWriter::new(File::create(path).unwrap());
    let o = SimpleFileOptions::default();
    if let Some(s) = structure {
        z.start_file("STRUCTURE.INFO", o).unwrap();
        z.write_all(s.as_bytes()).unwrap();
    }
    for (name, recs) in parts {
        z.start_file(*name, o).unwrap();
        for r in recs {
            z.write_all((r.join("\x04") + "\x04\r\n").as_bytes()).unwrap();
        }
    }
    z.finish().unwrap();
}

#[test]
fn edge_cases_and_options() {
    let dir = tempfile::tempdir().unwrap();
    let inpx = dir.path().join("edge.inpx");
    let st = "AUTHOR;GENRE;TITLE;SERIES;SERNO;FILE;SIZE;LIBID;DEL;EXT;DATE;LANG;LIBRATE;KEYWORDS;FOLDER;";
    write_inpx(
        &inpx,
        Some(st),
        &[
            (
                "a.inp",
                vec![
                    vec!["Ёлкин,Пётр,:Иванов,Иван,:", "sf_heroic:weird_code:", "«Ёлки»", "Лес", "2", "1", "10", "100", "0", "fb2", "2020-01-01", "ru", "3", "", ""],
                    vec!["елкин,петр,:", "", "Палки", "лес", "1", "2", "10", "101", "0", "FB2", "2020-01-02", "ru", "", "", ""],
                    vec!["Автор неизвестен,,:", "prose_classic:", "Безымянная", "", "", "3", "10", "102", "1", "fb2", "2020-01-03", "ru", "", "", "other.zip"],
                    // duplicate LIBID in another archive: stored under its file key
                    vec!["Сидоров,,:", "det_classic:", "Дубль", "", "", "4", "10", "100", "0", "fb2", "2020-01-04", "en", "", "", "dup.inp"],
                    // exact duplicate: dropped
                    vec!["Сидоров,,:", "det_classic:", "Дубль", "", "", "4", "10", "100", "0", "fb2", "2020-01-04", "en", "", "", "dup.inp"],
                    // plain file in a folder, no LIBID
                    vec![":", "", "Файл", "", "", "book", "10", "", "0", "epub", "bad-date", "", "", "", "plain/dir"],
                ],
            ),
            ("b.inp", vec![vec!["Петров,Пётр,Петрович:", "love_sf:", "Вторая", "", "", "9", "10", "200", "0", "fb2", "2021-05-05", "uk", "", "", ""]]),
        ],
    );
    let db = dir.path().join("lib_9.db");
    let s = import(&inpx, &db, None);
    assert_eq!(s.books, 6);
    assert_eq!(s.duplicate_lib_ids, 1);
    assert_eq!(s.dropped_duplicates, 1);
    let cat = Catalog::open(&db).unwrap();
    // "Ёлкин Пётр" and "елкин петр" are the same author; series "Лес"/"лес" merged.
    let authors = cat.authors().unwrap();
    let names: Vec<&str> = authors.rows.iter().map(|r| r.1.as_str()).collect();
    assert_eq!(names.iter().filter(|n| normalize(n) == "елкин петр").count(), 1);
    assert!(names.contains(&"Автор неизвестен"));
    let elkin = authors.rows.iter().find(|r| normalize(&r.1) == "елкин петр").unwrap();
    assert_eq!(elkin.2, 2);
    assert_eq!(cat.series_list().unwrap().rows.len(), 1);
    let ids = cat.ids_by_keys(&["lib:100".into(), "file:dup.zip/4.fb2".into(), "file:plain/dir/book.epub".into()]).unwrap();
    assert_eq!(ids.len(), 3);
    let plain = cat.book(ids.iter().find(|x| x.0.starts_with("file:plain")).unwrap().1).unwrap().unwrap();
    assert_eq!((plain.archive.as_str(), plain.folder.as_str()), ("", "plain/dir"));
    assert_eq!(plain.relative_path(), "plain/dir/book.epub");
    assert_eq!(plain.book.date, "");
    assert_eq!(plain.book.authors[0].name, "Автор неизвестен");
    let first = cat.book(1).unwrap().unwrap();
    assert_eq!(first.book.authors.len(), 2);
    assert_eq!(first.book.authors[0].name, "Ёлкин Пётр", "INPX author order kept");
    assert_eq!(first.stars, 3);
    assert_eq!(first.book.series.as_ref().unwrap().name, "Лес");
    assert_eq!(first.book.genres.len(), 2);
    assert!(first.book.genres.contains(&freelib_catalog::GENRE_OTHER), "unknown prefix → top-level Прочее");
    let del = cat.book(3).unwrap().unwrap();
    assert!(del.book.deleted);
    assert_eq!(del.archive, "other.zip");
    assert_eq!(cat.stats().live_book_count, 5);

    // skip_deleted + first_author_only
    let db2 = dir.path().join("lib_10.db");
    let opts = ImportOptions { inpx: inpx.clone(), db_path: db2.clone(), skip_deleted: true, first_author_only: true, ..Default::default() };
    let s2 = import_inpx(&opts, &no_progress, &AtomicBool::new(false)).unwrap();
    assert_eq!(s2.books, 5);
    let cat2 = Catalog::open(&db2).unwrap();
    assert!(cat2.stats().first_author_only && cat2.stats().skip_deleted);
    assert_eq!(cat2.book(1).unwrap().unwrap().book.authors.len(), 1);
    assert!(!cat2.authors().unwrap().rows.iter().any(|r| r.1.starts_with("Иванов")));

    // offsets with missing archives are skipped gracefully
    let s3 = import_inpx(
        &ImportOptions { inpx, db_path: dir.path().join("lib_11.db"), library_dir: Some(dir.path().join("nowhere")), resolve_offsets: true, ..Default::default() },
        &no_progress,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(s3.offsets_resolved, 0);
    assert_eq!(s3.missing_archives, vec!["a.zip", "b.zip", "dup.zip", "other.zip"]);
}

#[test]
fn schema_version_mismatch_and_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Catalog::open(dir.path().join("nope.db")).is_err());
    assert!(CatalogHandle::new(dir.path().join("nope.db")).get().is_none());
    let p = dir.path().join("old.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT); INSERT INTO meta VALUES ('schema_version','0');").unwrap();
    drop(c);
    assert!(matches!(Catalog::open(&p), Err(freelib_catalog::CatalogError::SchemaVersion { found: 0, .. })));
}

#[test]
fn app_db_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("app.db");
    let c = freelib_catalog::open_app_db(&p).unwrap();
    let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(v, freelib_catalog::APP_SCHEMA_VERSION);
    drop(c);
    // re-open is a no-op
    let c = freelib_catalog::open_app_db(&p).unwrap();
    c.execute("INSERT INTO setting(key, value) VALUES ('x', '1')", []).unwrap();
}
