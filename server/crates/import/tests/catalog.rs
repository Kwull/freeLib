//! End-to-end: generate an INPX (+ archives), import it, and exercise every catalog query.

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use freelib_catalog::{
    BookFilter, BookSelector, Catalog, CatalogHandle, Page, SearchKind, SearchQuery, genres,
    normalize,
};
use freelib_import::synth::{GenOptions, generate};
use freelib_import::{ImportError, ImportOptions, import_inpx, resolve_offsets};
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

fn all_pages(
    cat: &Catalog,
    sel: &BookSelector,
    f: &BookFilter,
    limit: usize,
) -> Vec<freelib_catalog::Book> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let p = cat
            .books(
                sel,
                f,
                &Page {
                    cursor: cursor.clone(),
                    limit,
                },
            )
            .unwrap();
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
    let gs = generate(
        &inpx,
        &GenOptions {
            showcase: false,
            books: 3000,
            per_archive: 500,
            seed: 11,
            files_dir: Some(lib.clone()),
            structure_info: true,
        },
    )
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
    assert!(
        st.collection_name
            .as_deref()
            .unwrap()
            .starts_with("Синтетическая")
    );
    assert!(st.imported_at.is_some() && st.catalog_version > 0);

    // --- authors / series lists
    for (list, count) in [
        (cat.authors().unwrap(), st.author_count),
        (cat.series_list().unwrap(), st.series_count),
    ] {
        // only names with live books are listed
        assert!(list.rows.len() as i64 <= count && list.rows.iter().all(|r| r.2 > 0));
        let keys: Vec<String> = list.rows.iter().map(|r| normalize(&r.1)).collect();
        // sorted by sort key, with the non-letter ("#") group moved to the end
        let hash_at = |k: &String| freelib_catalog::letter_of(k) == "#";
        let first_hash = keys.iter().position(hash_at).unwrap_or(keys.len());
        assert!(keys[first_hash..].iter().all(hash_at));
        assert!(
            keys[..first_hash].windows(2).all(|w| w[0] <= w[1]),
            "sorted by sort key"
        );
        assert!(keys[first_hash..].windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(
            list.letters.iter().map(|l| l.1).sum::<i64>(),
            list.rows.len() as i64
        );
        for (letter, _, first) in &list.letters {
            assert_eq!(&freelib_catalog::letter_of(&keys[*first as usize]), letter);
        }
    }
    let authors = cat.authors().unwrap();
    assert!(authors.rows.iter().map(|r| r.2).max().unwrap() > 5);

    // --- genres / languages
    let g = cat.genres("ru").unwrap();
    assert_eq!(g.len(), 322);
    for top in g.iter().filter(|x| x.parent == 0) {
        let kids: Vec<i64> = g
            .iter()
            .filter(|x| x.parent == top.id)
            .map(|x| x.count)
            .collect();
        // distinct books of the group: at least its largest child, at most the sum (+ direct books for "Прочее")
        assert!(
            top.count >= kids.iter().copied().max().unwrap_or(0),
            "group {}",
            top.id
        );
        if top.id != freelib_catalog::GENRE_OTHER {
            assert!(top.count <= kids.iter().sum::<i64>(), "group {}", top.id);
        }
    }
    assert!(
        g.iter().any(|x| x.id == 118 && x.count > 0),
        "unknown sf code → Фантастика: прочее"
    );
    let langs = cat.languages().unwrap();
    assert_eq!(langs[0].0, "ru");
    assert_eq!(langs.iter().map(|l| l.1).sum::<i64>(), st.live_book_count);

    // --- books by author, pagination and filters
    let top = authors.rows.iter().max_by_key(|r| r.2).unwrap();
    let all = all_pages(
        &cat,
        &BookSelector::Author(top.0),
        &BookFilter::default(),
        7,
    );
    assert_eq!(all.len() as i64, top.2);
    let first = cat
        .books(
            &BookSelector::Author(top.0),
            &BookFilter::default(),
            &Page {
                cursor: None,
                limit: 7,
            },
        )
        .unwrap();
    assert_eq!(first.total, top.2);
    assert_eq!(
        all.iter().map(|b| b.id).collect::<HashSet<_>>().len(),
        all.len(),
        "no duplicates across pages"
    );
    assert!(
        all.iter()
            .all(|b| b.authors.iter().any(|a| a.id == top.0) && !b.deleted)
    );
    // series books first, grouped and ordered by serno
    let series_keys: Vec<Option<String>> = all
        .iter()
        .map(|b| b.series.as_ref().map(|s| normalize(&s.name)))
        .collect();
    let first_none = series_keys
        .iter()
        .position(Option::is_none)
        .unwrap_or(series_keys.len());
    assert!(series_keys[first_none..].iter().all(Option::is_none));
    assert!(series_keys[..first_none].windows(2).all(|w| w[0] <= w[1]));
    let with_del = cat
        .books(
            &BookSelector::Author(top.0),
            &BookFilter {
                include_deleted: true,
                ..Default::default()
            },
            &Page::default(),
        )
        .unwrap();
    assert!(with_del.total >= top.2);
    let ru = cat
        .books(
            &BookSelector::Author(top.0),
            &BookFilter {
                langs: vec!["ru".into()],
                ext: Some("FB2".into()),
                include_deleted: false,
                q: None,
            },
            &Page::default(),
        )
        .unwrap();
    assert!(ru.books.iter().all(|b| b.lang == "ru" && b.ext == "fb2"));
    assert!(ru.total <= top.2);
    assert!(
        cat.books(
            &BookSelector::Author(top.0),
            &BookFilter::default(),
            &Page {
                cursor: Some("x".into()),
                limit: 5
            }
        )
        .is_err()
    );

    // --- books by series
    let series = cat.series_list().unwrap();
    let big_series = series.rows.iter().max_by_key(|r| r.2).unwrap();
    let sb = cat
        .books(
            &BookSelector::Series(big_series.0),
            &BookFilter::default(),
            &Page::default(),
        )
        .unwrap();
    assert_eq!(sb.total, big_series.2);
    let sernos: Vec<i64> = sb
        .books
        .iter()
        .map(|b| b.serno.unwrap_or(i64::MAX))
        .collect();
    assert!(sernos.windows(2).all(|w| w[0] <= w[1]));
    let info = cat.series(big_series.0).unwrap().unwrap();
    assert!(!info.authors.is_empty());

    // --- books by genre: leaf and group, both strategies agree
    let leaf = g
        .iter()
        .filter(|x| x.parent != 0)
        .max_by_key(|x| x.count)
        .unwrap();
    let lb = all_pages(
        &cat,
        &BookSelector::Genre(leaf.id),
        &BookFilter::default(),
        50,
    );
    assert_eq!(lb.len() as i64, leaf.count);
    assert!(lb.iter().all(|b| b.genres.contains(&leaf.id)));
    assert!(lb.windows(2).all(|w| w[0].date >= w[1].date));
    let group = g
        .iter()
        .filter(|x| x.parent == 0)
        .max_by_key(|x| x.count)
        .unwrap();
    let small = cat
        .books(
            &BookSelector::Genre(group.id),
            &BookFilter::default(),
            &Page {
                cursor: Some("10".into()),
                limit: 40,
            },
        )
        .unwrap();
    assert_eq!(small.total, group.count);
    let mut cat2 = Catalog::open(&db).unwrap();
    cat2.set_big_genre_threshold(1);
    let big = cat2
        .books(
            &BookSelector::Genre(group.id),
            &BookFilter::default(),
            &Page {
                cursor: Some("10".into()),
                limit: 40,
            },
        )
        .unwrap();
    assert_eq!(big.total, small.total);
    assert_eq!(big.books, small.books);
    let f_ru = BookFilter {
        langs: vec!["ru".into()],
        ..Default::default()
    };
    let a = cat
        .books(
            &BookSelector::Genre(group.id),
            &f_ru,
            &Page {
                cursor: None,
                limit: 5,
            },
        )
        .unwrap();
    let b = cat2
        .books(
            &BookSelector::Genre(group.id),
            &f_ru,
            &Page {
                cursor: None,
                limit: 5,
            },
        )
        .unwrap();
    assert_eq!((a.total, &a.books), (b.total, &b.books));
    assert_eq!(
        a.total as usize,
        all_pages(&cat, &BookSelector::Genre(group.id), &f_ru, 500).len()
    );
    let kids = genres().with_descendants(group.id);
    assert!(
        a.books
            .iter()
            .all(|bk| bk.genres.iter().any(|x| kids.contains(x)))
    );

    // --- since
    let since = "2020-01-01";
    let sp = cat
        .books(
            &BookSelector::Since(since.into()),
            &BookFilter::default(),
            &Page {
                cursor: None,
                limit: 100,
            },
        )
        .unwrap();
    assert!(sp.books.iter().all(|b| b.date.as_str() >= since));
    assert!(sp.books.windows(2).all(|w| w[0].date >= w[1].date));
    assert_eq!(
        sp.total as usize,
        all_pages(
            &cat,
            &BookSelector::Since(since.into()),
            &BookFilter::default(),
            1000
        )
        .len()
    );
    assert_eq!(cat.count_newer_than("2019-12-31").unwrap(), sp.total);

    // --- ids (shelves) and keys
    let ids: Vec<i64> = vec![5, 1, 2999, 77, 123_456];
    let ib = cat
        .books(
            &BookSelector::Ids(ids.clone()),
            &BookFilter {
                include_deleted: true,
                ..Default::default()
            },
            &Page::default(),
        )
        .unwrap();
    assert_eq!(ib.total, 4);
    let by_ids = cat.books_by_ids(&ids).unwrap();
    assert_eq!(
        by_ids.iter().map(|b| b.id).collect::<Vec<_>>(),
        vec![5, 1, 2999, 77]
    );
    let keys = cat.keys_by_ids(&[1, 2, 3]).unwrap();
    assert_eq!(keys.len(), 3);
    let back = cat
        .ids_by_keys(
            &keys
                .iter()
                .map(|k| k.1.clone())
                .chain(["lib:nope".to_string()])
                .collect::<Vec<_>>(),
        )
        .unwrap();
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
    let word = probe
        .title
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| w.chars().count() >= 3)
        .unwrap()
        .to_string();
    let r = cat
        .search(&SearchQuery {
            q: word.clone(),
            include_deleted: true,
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    assert!(r.total > 0);
    assert!(
        r.books.iter().any(|b| b.id == 42) || r.total > 1000,
        "{word}"
    );
    assert_eq!(r.facets.lang.iter().map(|x| x.1).sum::<i64>(), r.total);
    assert_eq!(r.facets.ext.iter().map(|x| x.1).sum::<i64>(), r.total);
    // prefix + author word
    let last = &probe.authors[0].name.split(' ').next().unwrap().to_string();
    let prefix: String = last.chars().take(4).collect();
    let r = cat
        .search(&SearchQuery {
            q: prefix.clone(),
            limit: 50,
            ..Default::default()
        })
        .unwrap();
    assert!(!r.authors.is_empty(), "author prefix {prefix}");
    assert!(r.authors.iter().all(|a| {
        normalize(&a.name)
            .split(' ')
            .any(|w| w.starts_with(&normalize(&prefix)))
    }));
    let ra = cat
        .search(&SearchQuery {
            q: prefix.clone(),
            kind: SearchKind::Authors,
            ..Default::default()
        })
        .unwrap();
    assert!(ra.books.is_empty() && !ra.authors.is_empty());
    // filters narrow; facets ignore their own dimension
    let rf = cat
        .search(&SearchQuery {
            q: prefix.clone(),
            langs: vec!["en".into()],
            limit: 50,
            ..Default::default()
        })
        .unwrap();
    assert!(rf.books.iter().all(|b| b.lang == "en"));
    assert_eq!(rf.facets.lang, r.facets.lang);
    let rg = cat
        .search(&SearchQuery {
            q: prefix.clone(),
            genres: vec![group.id],
            limit: 50,
            ..Default::default()
        })
        .unwrap();
    assert!(
        rg.books
            .iter()
            .all(|b| b.genres.iter().any(|x| kids.contains(x)))
    );
    let rd = cat
        .search(&SearchQuery {
            q: prefix,
            from: Some("2015-01-01".into()),
            to: Some("2016-12-31".into()),
            limit: 50,
            ..Default::default()
        })
        .unwrap();
    assert!(
        rd.books
            .iter()
            .all(|b| b.date.as_str() >= "2015-01-01" && b.date.as_str() <= "2016-12-31")
    );
    // series search
    let sname = &info.name;
    let rs = cat
        .search(&SearchQuery {
            q: sname.clone(),
            kind: SearchKind::Series,
            ..Default::default()
        })
        .unwrap();
    assert!(rs.series.iter().any(|s| s.id == info.id), "{sname}");
    // ё / е equivalence and too-short queries
    let yo = cat
        .search(&SearchQuery {
            q: "Тёмный".into(),
            limit: 5,
            ..Default::default()
        })
        .unwrap();
    let ye = cat
        .search(&SearchQuery {
            q: "темный".into(),
            limit: 5,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(yo.total, ye.total);
    assert!(yo.total > 0);
    assert_eq!(
        cat.search(&SearchQuery {
            q: "а".into(),
            ..Default::default()
        })
        .unwrap()
        .total,
        0
    );

    // --- re-import and reload: version changes, old handle keeps working
    let v1 = cat.catalog_version();
    import(&inpx, &db, None);
    let cat_new = handle.reload().unwrap();
    assert!(cat_new.catalog_version() > v1);
    assert_eq!(
        cat.books_by_ids(&[1]).unwrap().len(),
        1,
        "old catalog still readable"
    );
    {
        // Drain the idle pool; the next connection would come from the replaced file.
        let mut held = Vec::new();
        let err = loop {
            match cat.conn() {
                Ok(c) => held.push(c),
                Err(e) => break e,
            }
            assert!(held.len() < 20);
        };
        assert!(matches!(err, freelib_catalog::CatalogError::Stale));
    }
    assert_eq!(
        cat_new.book(10).unwrap().unwrap().arch_offset,
        None,
        "no offsets without library dir"
    );

    // --- standalone offset resolution
    drop(cat_new);
    handle.close();
    let os = resolve_offsets(&db, &lib, &no_progress, &AtomicBool::new(false)).unwrap();
    assert_eq!(os.resolved, 3000);
    assert_eq!(
        handle
            .reload()
            .unwrap()
            .book(10)
            .unwrap()
            .unwrap()
            .arch_offset,
        d.arch_offset
    );

    // --- cancellation leaves the current catalog alone
    let cancel = AtomicBool::new(true);
    let before = handle.get().unwrap().catalog_version();
    let r = import_inpx(
        &ImportOptions {
            inpx: inpx.clone(),
            db_path: db.clone(),
            ..Default::default()
        },
        &no_progress,
        &cancel,
    );
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
            z.write_all((r.join("\x04") + "\x04\r\n").as_bytes())
                .unwrap();
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
                    vec![
                        "Ёлкин,Пётр,:Иванов,Иван,:",
                        "sf_heroic:weird_code:",
                        "«Ёлки»",
                        "Лес",
                        "2",
                        "1",
                        "10",
                        "100",
                        "0",
                        "fb2",
                        "2020-01-01",
                        "ru",
                        "3",
                        "",
                        "",
                    ],
                    vec![
                        "елкин,петр,:",
                        "",
                        "Палки",
                        "лес",
                        "1",
                        "2",
                        "10",
                        "101",
                        "0",
                        "FB2",
                        "2020-01-02",
                        "ru",
                        "",
                        "",
                        "",
                    ],
                    vec![
                        "Автор неизвестен,,:",
                        "prose_classic:",
                        "Безымянная",
                        "",
                        "",
                        "3",
                        "10",
                        "102",
                        "1",
                        "fb2",
                        "2020-01-03",
                        "ru",
                        "",
                        "",
                        "other.zip",
                    ],
                    // duplicate LIBID in another archive: stored under its file key
                    vec![
                        "Сидоров,,:",
                        "det_classic:",
                        "Дубль",
                        "",
                        "",
                        "4",
                        "10",
                        "100",
                        "0",
                        "fb2",
                        "2020-01-04",
                        "en",
                        "",
                        "",
                        "dup.inp",
                    ],
                    // exact duplicate: dropped
                    vec![
                        "Сидоров,,:",
                        "det_classic:",
                        "Дубль",
                        "",
                        "",
                        "4",
                        "10",
                        "100",
                        "0",
                        "fb2",
                        "2020-01-04",
                        "en",
                        "",
                        "",
                        "dup.inp",
                    ],
                    // plain file in a folder, no LIBID
                    vec![
                        ":",
                        "",
                        "Файл",
                        "",
                        "",
                        "book",
                        "10",
                        "",
                        "0",
                        "epub",
                        "bad-date",
                        "",
                        "",
                        "",
                        "plain/dir",
                    ],
                ],
            ),
            (
                "b.inp",
                vec![vec![
                    "Петров,Пётр,Петрович:",
                    "love_sf:",
                    "Вторая",
                    "",
                    "",
                    "9",
                    "10",
                    "200",
                    "0",
                    "fb2",
                    "2021-05-05",
                    "uk",
                    "",
                    "",
                    "",
                ]],
            ),
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
    assert_eq!(
        names
            .iter()
            .filter(|n| normalize(n) == "елкин петр")
            .count(),
        1
    );
    assert!(names.contains(&"Автор неизвестен"));
    let elkin = authors
        .rows
        .iter()
        .find(|r| normalize(&r.1) == "елкин петр")
        .unwrap();
    assert_eq!(elkin.2, 2);
    assert_eq!(cat.series_list().unwrap().rows.len(), 1);
    let ids = cat
        .ids_by_keys(&[
            "lib:100".into(),
            "file:dup.zip/4.fb2".into(),
            "file:plain/dir/book.epub".into(),
        ])
        .unwrap();
    assert_eq!(ids.len(), 3);
    let plain = cat
        .book(
            ids.iter()
                .find(|x| x.0.starts_with("file:plain"))
                .unwrap()
                .1,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        (plain.archive.as_str(), plain.folder.as_str()),
        ("", "plain/dir")
    );
    assert_eq!(plain.relative_path(), "plain/dir/book.epub");
    assert_eq!(plain.book.date, "");
    assert_eq!(plain.book.authors[0].name, "Автор неизвестен");
    let first = cat.book(1).unwrap().unwrap();
    assert_eq!(first.book.authors.len(), 2);
    assert_eq!(
        first.book.authors[0].name, "Ёлкин Пётр",
        "INPX author order kept"
    );
    assert_eq!(first.stars, 3);
    assert_eq!(first.book.series.as_ref().unwrap().name, "Лес");
    assert_eq!(first.book.genres.len(), 2);
    assert!(
        first.book.genres.contains(&freelib_catalog::GENRE_OTHER),
        "unknown prefix → top-level Прочее"
    );
    let del = cat.book(3).unwrap().unwrap();
    assert!(del.book.deleted);
    assert_eq!(del.archive, "other.zip");
    assert_eq!(cat.stats().live_book_count, 5);

    // skip_deleted + first_author_only
    let db2 = dir.path().join("lib_10.db");
    let opts = ImportOptions {
        inpx: inpx.clone(),
        db_path: db2.clone(),
        skip_deleted: true,
        first_author_only: true,
        ..Default::default()
    };
    let s2 = import_inpx(&opts, &no_progress, &AtomicBool::new(false)).unwrap();
    assert_eq!(s2.books, 5);
    let cat2 = Catalog::open(&db2).unwrap();
    assert!(cat2.stats().first_author_only && cat2.stats().skip_deleted);
    assert_eq!(cat2.book(1).unwrap().unwrap().book.authors.len(), 1);
    assert!(
        !cat2
            .authors()
            .unwrap()
            .rows
            .iter()
            .any(|r| r.1.starts_with("Иванов"))
    );

    // offsets with missing archives are skipped gracefully
    let s3 = import_inpx(
        &ImportOptions {
            inpx,
            db_path: dir.path().join("lib_11.db"),
            library_dir: Some(dir.path().join("nowhere")),
            resolve_offsets: true,
            ..Default::default()
        },
        &no_progress,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(s3.offsets_resolved, 0);
    assert_eq!(
        s3.missing_archives,
        vec!["a.zip", "b.zip", "dup.zip", "other.zip"]
    );
}

#[test]
fn schema_version_mismatch_and_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Catalog::open(dir.path().join("nope.db")).is_err());
    assert!(
        CatalogHandle::new(dir.path().join("nope.db"))
            .get()
            .is_none()
    );
    let p = dir.path().join("old.db");
    let c = rusqlite::Connection::open(&p).unwrap();
    c.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT); INSERT INTO meta VALUES ('schema_version','0');").unwrap();
    drop(c);
    assert!(matches!(
        Catalog::open(&p),
        Err(freelib_catalog::CatalogError::SchemaVersion { found: 0, .. })
    ));
}

#[test]
fn app_db_migrates() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("app.db");
    let c = freelib_catalog::open_app_db(&p).unwrap();
    let v: i64 = c
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(v, freelib_catalog::APP_SCHEMA_VERSION);
    drop(c);
    // re-open is a no-op
    let c = freelib_catalog::open_app_db(&p).unwrap();
    c.execute("INSERT INTO setting(key, value) VALUES ('x', '1')", [])
        .unwrap();
}

#[test]
fn name_lists_fold_diacritics_and_put_non_letters_last() {
    let dir = tempfile::tempdir().unwrap();
    let inpx = dir.path().join("names.inpx");
    let st = "AUTHOR;GENRE;TITLE;SERIES;SERNO;FILE;SIZE;LIBID;DEL;EXT;DATE;LANG;LIBRATE;KEYWORDS;";
    let rec = |id: usize, author: &str, title: &str, series: &str, del: bool| -> Vec<String> {
        vec![
            author.to_string(),
            "sf:".into(),
            title.into(),
            series.into(),
            if series.is_empty() { "" } else { "1" }.into(),
            id.to_string(),
            "10".into(),
            id.to_string(),
            if del { "1" } else { "0" }.into(),
            "fb2".into(),
            format!("2020-01-{:02}", id),
            "ru".into(),
            "".into(),
            "".into(),
        ]
    };
    let recs: Vec<Vec<String>> = vec![
        rec(1, "Čapek,Karel,:", "Válka s mloky", "", false),
        rec(2, "Capek,Anna,:", "Solo", "", false),
        rec(3, "Ødegaard,Knut,:", "Nord", "Øst", false),
        rec(4, "Zeta,,:", "Z", "Ostrov", false),
        rec(5, "Абрамов,Фёдор,:", "Дом", "12 стульев", false),
        rec(6, "1984 Group,,:", "Numbers", "", false),
        rec(7, "#55 Aircraft,,:", "Planes", "", false),
        rec(8, "Удалённый,Автор,:", "Gone", "Пустая серия", true),
        // an anthology (4 authors) and a two-author book
        rec(
            9,
            "Абрамов,Фёдор,:Capek,Anna,:Zeta,,:1984 Group,,:",
            "Антология",
            "",
            false,
        ),
        rec(10, "Абрамов,Фёдор,:Zeta,,:", "Вдвоём", "", false),
        rec(
            11,
            "Абрамов,Фёдор,:Zeta,,:",
            "Вдвоём-2",
            "12 стульев",
            false,
        ),
    ];
    let recs: Vec<Vec<&str>> = recs
        .iter()
        .map(|r| r.iter().map(String::as_str).collect())
        .collect();
    write_inpx(&inpx, Some(st), &[("a.inp", recs)]);
    let db = dir.path().join("lib_3.db");
    import(&inpx, &db, None);
    let cat = Catalog::open(&db).unwrap();

    let a = cat.authors().unwrap();
    let names: Vec<&str> = a.rows.iter().map(|r| r.1.as_str()).collect();
    assert_eq!(
        names,
        [
            "Capek Anna",
            "Čapek Karel",
            "Ødegaard Knut",
            "Zeta",
            "Абрамов Фёдор",
            "1984 Group",
            "#55 Aircraft"
        ],
        "diacritics folded, deleted-only author hidden, # group last"
    );
    let letters: Vec<(&str, i64, i64)> = a
        .letters
        .iter()
        .map(|(l, c, p)| (l.as_str(), *c, *p))
        .collect();
    assert_eq!(
        letters,
        [
            ("C", 2, 0),
            ("O", 1, 2),
            ("Z", 1, 3),
            ("А", 1, 4),
            ("#", 2, 5)
        ]
    );
    let s = cat.series_list().unwrap();
    let snames: Vec<&str> = s.rows.iter().map(|r| r.1.as_str()).collect();
    assert_eq!(snames, ["Øst", "Ostrov", "12 стульев"]);
    assert_eq!(s.letters.last().unwrap().0, "#");

    // author summary and co-authors
    let abr = a.rows.iter().find(|r| r.1 == "Абрамов Фёдор").unwrap().0;
    let sum = cat.author_summary(abr).unwrap().unwrap();
    assert_eq!((sum.count, sum.anthologies, sum.without_series), (4, 1, 2));
    assert_eq!(sum.series.len(), 1);
    assert_eq!(
        (sum.series[0].name.as_str(), sum.series[0].count),
        ("12 стульев", 2)
    );
    assert_eq!(sum.langs, vec![("ru".to_string(), 4)]);
    assert_eq!(
        (sum.first_date.as_str(), sum.last_date.as_str()),
        ("2020-01-05", "2020-01-11")
    );
    assert_eq!(sum.coauthor_count, 3);
    // Zeta: 2 direct + the anthology; Capek and 1984 Group share only the anthology
    assert_eq!(sum.coauthors.len(), 1);
    assert_eq!(
        (
            sum.coauthors[0].name.as_str(),
            sum.coauthors[0].books,
            sum.coauthors[0].direct
        ),
        ("Zeta", 3, 2)
    );
    let all = cat.coauthors(abr).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all[0].name, "Zeta");
    assert!(all[1..].iter().all(|c| c.books == 1 && c.direct == 0));
    assert!(cat.author_summary(9999).unwrap().is_none());

    // text filter on book lists
    let f = freelib_catalog::BookFilter {
        q: Some("вдвоём".into()),
        ..Default::default()
    };
    let p = cat
        .books(&BookSelector::Author(abr), &f, &Page::default())
        .unwrap();
    let titles: Vec<&str> = p.books.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(titles, ["Вдвоём-2", "Вдвоём"]);
    assert_eq!(p.total, 2);
    let p = cat
        .books(
            &BookSelector::Since("2000-01-01".into()),
            &freelib_catalog::BookFilter {
                q: Some("capek".into()),
                ..Default::default()
            },
            &Page {
                cursor: None,
                limit: 1,
            },
        )
        .unwrap();
    assert_eq!(
        p.total, 3,
        "author names match too (Čapek, Capek, anthology)"
    );
    assert!(p.next_cursor.is_some());
}

#[test]
fn showcase_editions_are_grouped() {
    let dir = tempfile::tempdir().unwrap();
    let inpx = dir.path().join("lib.inpx");
    generate(
        &inpx,
        &GenOptions {
            showcase: true,
            books: 300,
            per_archive: 500,
            seed: 3,
            files_dir: None,
            structure_info: true,
        },
    )
    .unwrap();
    let db = dir.path().join("lib_1.db");
    let opts = ImportOptions {
        inpx,
        db_path: db.clone(),
        ..Default::default()
    };
    let stats = import_inpx(&opts, &|_, _, _: &str| {}, &AtomicBool::new(false)).unwrap();
    assert!(
        stats.series_works_joined >= 10,
        "{}",
        stats.series_works_joined
    );
    let conn = rusqlite::Connection::open(&db).unwrap();
    let work = |title: &str| -> Vec<i64> {
        conn.prepare("SELECT work_id FROM book WHERE title=?1 AND id IN (SELECT book_id FROM book_author ba JOIN author a ON a.id=ba.author_id WHERE a.last IN ('Азимов','Маринина'))")
            .unwrap()
            .query_map([title], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    let one = |t: &str| {
        let v = work(t);
        assert!(!v.is_empty(), "{t}");
        v[0]
    };
    for group in [
        &["Второй Фонд", "Дублеры"][..],
        &[
            "Академия на краю гибели",
            "Академия на краю гибели (fb2)",
            "Край Основания",
            "Миры Айзека Азимова. Книга 9",
            "Сообщество на краю",
        ],
        &[
            "Академия и Земля",
            "Миры Айзека Азимова. Книга 10",
            "Основание и Земля",
            "Сообщество и Земля",
        ],
        &["Люди за спиной. Том 1", "Люди за спиной, том 1"],
        &["Фонд", "Фонд [litres]"],
    ] {
        let w = one(group[0]);
        for t in group {
            assert!(
                work(t).iter().all(|&x| x == w),
                "{t} not in the work of {}",
                group[0]
            );
        }
    }
    assert_ne!(one("Люди за спиной. Том 1"), one("Люди за спиной. Том 2"));
    assert_ne!(one("Страхи Академии"), one("Академия и Хаос"));
    assert_ne!(one("Академия на краю гибели"), one("Академия и Земля"));
    for t in [
        "Академия. Книги 1-7",
        "Академия. Начало",
        "Академия. Первая трилогия",
        "Миры Айзека Азимова. Книга 7",
        "Путь к Академии",
    ] {
        let w = one(t);
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM book WHERE work_id=?1", [w], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1, "{t}");
    }
}
