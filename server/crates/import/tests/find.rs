//! Russian-aware search (word forms, transliteration, typos, ranking), editions of one work,
//! and the start page ("Continue series", "New from authors") on a small hand-made catalog.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use freelib_catalog::home::NewFromSeeds;
use freelib_catalog::{
    BookFilter, BookSelector, Catalog, NoRatings, Page, RatingQuery, SearchKind, SearchQuery,
};
use freelib_import::{ImportOptions, import_inpx};
use zip::write::SimpleFileOptions;

const STRUG: &str = "Стругацкий,Аркадий,Натанович:Стругацкий,Борис,Натанович:";
const ASIMOV: &str = "Азимов,Айзек,:";

/// (authors, title, series, serno, size, ext, date, lang, keywords)
type Rec = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

// Book ids follow this order (1-based).
const BOOKS: &[Rec] = &[
    (
        STRUG,
        "Пикник на обочине",
        "",
        "",
        "500000",
        "fb2",
        "2020-01-01",
        "ru",
        "",
    ), // 1
    (
        STRUG,
        "Пикник на обочине (другой перевод)",
        "",
        "",
        "900000",
        "epub",
        "2021-01-01",
        "ru",
        "перевод М. Иванова",
    ), // 2
    (
        STRUG,
        "Пикник на обочине",
        "",
        "",
        "800000",
        "fb2",
        "2019-01-01",
        "ru",
        "",
    ), // 3
    (
        ASIMOV,
        "Книга о книгах",
        "Основание",
        "1",
        "300000",
        "fb2",
        "2018-01-01",
        "ru",
        "",
    ), // 4
    (
        ASIMOV,
        "Книги будущего",
        "Основание",
        "2",
        "300000",
        "fb2",
        "2018-02-01",
        "ru",
        "",
    ), // 5
    (
        ASIMOV,
        "Основание и империя",
        "Основание",
        "3",
        "300000",
        "fb2",
        "2024-03-01",
        "ru",
        "",
    ), // 6
    (
        ASIMOV,
        "Второе основание",
        "Основание",
        "4",
        "300000",
        "fb2",
        "2024-04-01",
        "ru",
        "",
    ), // 7
    (
        "Иванов,Иван,:",
        "Война и мир",
        "",
        "",
        "100000",
        "fb2",
        "2010-01-01",
        "ru",
        "",
    ), // 8
    (
        "Петров,Пётр,:",
        "Мир и война",
        "",
        "",
        "100000",
        "fb2",
        "2010-01-01",
        "ru",
        "",
    ), // 9
    (
        "Сидоров,Сидор,:",
        "Войны миров",
        "",
        "",
        "100000",
        "fb2",
        "2010-01-01",
        "ru",
        "",
    ), // 10
    (
        "Толстой,Лев,:",
        "Избранное",
        "",
        "",
        "100000",
        "fb2",
        "2010-01-01",
        "ru",
        "",
    ), // 11
    (
        "Толстой,Лев,:",
        "Избранное",
        "",
        "",
        "200000",
        "fb2",
        "2011-01-01",
        "ru",
        "",
    ), // 12
    (
        "",
        "Сборник сказок",
        "",
        "",
        "100000",
        "fb2",
        "2010-01-01",
        "ru",
        "",
    ), // 13
    (
        "",
        "Сборник сказок",
        "",
        "",
        "100000",
        "fb2",
        "2011-01-01",
        "ru",
        "",
    ), // 14
    (
        "Lem,Stanisław,:",
        "Solaris",
        "",
        "",
        "100000",
        "fb2",
        "2010-01-01",
        "en",
        "",
    ), // 15
    (
        "Кузнецов,Кузьма,:",
        "Записки",
        "",
        "",
        "100000",
        "fb2",
        "2010-01-01",
        "ru",
        "азимвв",
    ), // 16
];

fn build() -> (tempfile::TempDir, Catalog) {
    let dir = tempfile::tempdir().unwrap();
    let inpx = dir.path().join("find.inpx");
    let mut z = zip::ZipWriter::new(File::create(&inpx).unwrap());
    z.start_file("a.inp", SimpleFileOptions::default()).unwrap();
    for (i, b) in BOOKS.iter().enumerate() {
        // AUTHOR;GENRE;TITLE;SERIES;SERNO;FILE;SIZE;LIBID;DEL;EXT;DATE;LANG;STARS;KEYWORDS
        let lib_id = (100 + i).to_string();
        let file = (i + 1).to_string();
        let rec = [
            b.0, "sf:", b.1, b.2, b.3, &file, b.4, &lib_id, "0", b.5, b.6, b.7, "0", b.8,
        ];
        z.write_all((rec.join("\x04") + "\x04\r\n").as_bytes())
            .unwrap();
    }
    z.finish().unwrap();
    let db = dir.path().join("lib_1.db");
    import_inpx(
        &ImportOptions {
            inpx,
            db_path: db.clone(),
            ..Default::default()
        },
        &|_, _, _| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    let cat = Catalog::open(Path::new(&db)).unwrap();
    (dir, cat)
}

fn search(cat: &Catalog, q: &str, group: bool) -> freelib_catalog::SearchResult {
    cat.search(&SearchQuery {
        q: q.into(),
        limit: 100,
        group,
        ..Default::default()
    })
    .unwrap()
}

fn ids(r: &freelib_catalog::SearchResult) -> Vec<i64> {
    r.books.iter().map(|b| b.id).collect()
}

#[test]
fn word_forms_translit_typos_and_ranking() {
    let (_d, cat) = build();

    // word forms: книгу / книгой → Книга, Книги
    for q in ["книгу", "книгой", "книга"] {
        let found = ids(&search(&cat, q, false));
        assert!(found.contains(&4) && found.contains(&5), "{q}: {found:?}");
    }
    // the exact prefix ranks above a stem-only match
    assert_eq!(ids(&search(&cat, "книги", false))[..2], [5, 4]);

    // phrase > all words as prefixes > word forms
    let r = ids(&search(&cat, "война и мир", false));
    let pos = |id: i64| r.iter().position(|x| *x == id).unwrap();
    assert!(pos(8) < pos(9) && pos(9) < pos(10), "{r:?}");

    // transliteration, both ways
    for q in ["strugatsky", "strugackie", "Strugatskii"] {
        let r = search(&cat, q, false);
        assert!(ids(&r).contains(&1), "{q}");
        assert!(
            r.authors.iter().any(|a| a.name.starts_with("Стругацкий")),
            "{q}"
        );
    }
    let r = search(&cat, "azimov", false);
    assert_eq!(r.authors[0].name, "Азимов Айзек");
    assert!(
        r.highlight.contains(&"азимов".to_string()),
        "{:?}",
        r.highlight
    );
    assert!(ids(&search(&cat, "лем", false)).contains(&15));
    // ё = е
    assert!(ids(&search(&cat, "петр", false)).contains(&9));

    // a missing letter that transliterates the same still finds the book
    assert!(ids(&search(&cat, "Стругацкй", false)).contains(&1));
    // typos: nothing found as typed → corrected query
    let r = search(&cat, "Азмов", false);
    assert_eq!(r.corrected.as_deref(), Some("азимов"));
    assert_eq!(r.authors[0].name, "Азимов Айзек");
    let r = search(&cat, "пикнк", false);
    assert_eq!(r.corrected.as_deref(), Some("пикник"));
    assert!(ids(&r).contains(&1));
    // little found (a keyword) → a suggestion only
    let r = search(&cat, "азимв", false);
    assert_eq!(ids(&r), [16]);
    assert!(r.corrected.is_none());
    assert_eq!(r.did_you_mean.as_deref(), Some("азимов"));
    // stems are highlighted as whole words
    let r = search(&cat, "книгу", false);
    assert!(
        r.highlight.contains(&"книга".to_string()),
        "{:?}",
        r.highlight
    );
    assert!(r.highlight.contains(&"книгах".to_string()));
    // author search by a word form
    let r = cat
        .search(&SearchQuery {
            q: "стругацкие".into(),
            kind: SearchKind::Authors,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(r.authors.len(), 2);
}

#[test]
fn editions_of_one_work() {
    let (_d, cat) = build();
    // three editions of "Пикник на обочине" → one row, best copy = the larger FB2
    let r = search(&cat, "пикник", true);
    assert_eq!(ids(&r), [3]);
    assert_eq!(r.total, 1);
    let e = r.books[0].editions.as_ref().unwrap();
    assert_eq!(e.count, 3);
    assert_eq!(e.ids[0], 3);
    assert_eq!(search(&cat, "пикник", false).books.len(), 3);

    let page = |sel: BookSelector| {
        cat.books_page(
            &sel,
            &BookFilter::default(),
            &RatingQuery::default(),
            &NoRatings,
            &Page::default(),
            true,
        )
        .unwrap()
    };
    let strug = cat
        .search(&SearchQuery {
            q: "стругацкий аркадий".into(),
            kind: SearchKind::Authors,
            ..Default::default()
        })
        .unwrap()
        .authors[0]
        .id;
    let p = page(BookSelector::Author(strug));
    assert_eq!(p.total, 1);
    assert_eq!(p.books[0].id, 3);
    // generic titles and unknown authors are never grouped
    let tolstoy = cat
        .search(&SearchQuery {
            q: "толстой".into(),
            kind: SearchKind::Authors,
            ..Default::default()
        })
        .unwrap()
        .authors[0]
        .id;
    assert_eq!(page(BookSelector::Author(tolstoy)).total, 2);
    assert_eq!(search(&cat, "сборник сказок", true).total, 2);

    // editions: best first, with the note from the title / keywords
    let eds = cat.editions(1, false, &NoRatings).unwrap().unwrap();
    let order: Vec<i64> = eds.iter().map(|e| e.book.id).collect();
    assert_eq!(order, [3, 1, 2]);
    assert_eq!(
        eds[2].note.as_deref(),
        Some("другой перевод; перевод М. Иванова")
    );
    assert!(eds[0].note.is_none());
    assert!(cat.editions(9999, false, &NoRatings).unwrap().is_none());

    // a known cover beats format and size
    struct Cover;
    impl freelib_catalog::RatingSource for Cover {
        fn my(&self, _: i64) -> u8 {
            0
        }
        fn ext(&self, _: i64) -> Option<(u16, u32)> {
            None
        }
        fn has_cover(&self, id: i64) -> Option<bool> {
            Some(id == 2)
        }
    }
    let eds = cat.editions(1, false, &Cover).unwrap().unwrap();
    assert_eq!(eds[0].book.id, 2);
}

#[test]
fn continue_series_and_new_from_authors() {
    let (_d, cat) = build();
    let done = |v: &[i64]| -> HashMap<i64, String> {
        v.iter()
            .map(|id| (*id, format!("2026-01-0{id}T00:00:00Z")))
            .collect()
    };
    let none = HashSet::new();
    let next = |d: &[i64]| -> Vec<Vec<i64>> {
        cat.continue_series(&done(d), &none, &NoRatings, 2, 10)
            .unwrap()
            .iter()
            .map(|s| s.next.iter().map(|b| b.id).collect())
            .collect()
    };
    assert_eq!(next(&[4]), [vec![5, 6]]);
    // after the last one done, not the gaps before it
    assert_eq!(next(&[4, 6]), [vec![7]]);
    // finished series and books without a series are left out
    assert!(next(&[7]).is_empty());
    assert!(next(&[1, 8]).is_empty());
    let s = &cat
        .continue_series(&done(&[5]), &none, &NoRatings, 2, 10)
        .unwrap()[0];
    assert_eq!(
        (s.series.name.as_str(), s.works, s.done),
        ("Основание", 4, 1)
    );
    // dismissed
    let dismissed: HashSet<String> = ["основание".to_string()].into();
    assert!(
        cat.continue_series(&done(&[4]), &dismissed, &NoRatings, 2, 10)
            .unwrap()
            .is_empty()
    );

    // new from authors the user read: newer books by Asimov, not the one read
    let seeds = NewFromSeeds {
        read_authors: cat.reading_authors(&[4]).unwrap(),
        ..Default::default()
    };
    assert_eq!(seeds.read_authors.len(), 1);
    let exclude: HashSet<i64> = [4].into();
    let (books, total) = cat
        .new_from(&seeds, "2024-01-01", &exclude, &NoRatings, 10)
        .unwrap();
    assert_eq!(total, 2);
    assert_eq!(books.iter().map(|b| b.book.id).collect::<Vec<_>>(), [7, 6]);
    assert!(!books[0].reason.followed);
    assert_eq!(books[0].reason.kind, "author");
    // a followed series
    let series = cat
        .search(&SearchQuery {
            q: "основание".into(),
            kind: SearchKind::Series,
            ..Default::default()
        })
        .unwrap()
        .series[0]
        .id;
    let seeds = NewFromSeeds {
        followed_series: vec![series],
        ..Default::default()
    };
    let (books, _) = cat
        .new_from(&seeds, "2018-01-15", &HashSet::new(), &NoRatings, 10)
        .unwrap();
    assert_eq!(books.len(), 3);
    assert_eq!(books[0].reason.kind, "series");
    assert!(books[0].reason.followed);
    // keys resolve back to ids
    assert_eq!(
        cat.series_by_keys(&["основание".into()]).unwrap(),
        [("основание".to_string(), series)]
    );
}
