//! Editions of one work shown as one row, with the best copy picked automatically.
//!
//! The importer gives every book a `work_id` (the smallest book id of the work) by two rules:
//!
//! 1. **Title rule:** same language, title key ([`crate::text::work_title_key_in_series`]:
//!    normalized title without trailing edition notes such as `(другой перевод)`, `(fb2)`,
//!    `[litres]`, or a trailing `Книга N` that only repeats the series number) and author set
//!    (in any order).
//! 2. **Series-number rule** ([`series_number_merges`]): same language, same series, same
//!    series number (> 0), same first author and compatible author sets (one contains the
//!    other) — different translations of one novel under different titles («Академия на краю
//!    гибели», «Край Основания», «Сообщество на краю» as #6). Not applied when the titles name
//!    different volumes (`Том 1` / `Том 2`), nor in series whose numbering looks unreliable.
//!
//! Books by an unknown author and generic titles ("Избранное", "Рассказы") keep their own id.
//! A list is grouped in memory: each work appears once, at the position of its first edition
//! in the list, represented by its best copy.
//!
//! **Best copy** (first difference wins):
//! 1. not deleted;
//! 2. its title names no volume (`Миры Айзека Азимова. Книга 9` joined by the series-number
//!    rule is an omnibus: a plain copy of the novel represents the work better);
//! 3. has a cover (as far as the server knows — covers are only known for books whose preview
//!    was extracted; unknown ranks between yes and no);
//! 4. format: FB2 (converts best, usually with its cover), then EPUB, then anything else;
//! 5. larger file, compared in 20 % steps and capped at 30 MB (a bigger file usually has the
//!    illustrations and the complete text; beyond the cap size says nothing);
//! 6. newer date (a later upload is usually a corrected version);
//! 7. higher library rating;
//! 8. lower id (stable).

use std::cmp::Ordering;
use std::collections::HashMap;

use rusqlite::OptionalExtension;

use crate::catalog::{BookFilter, BookSelector, Catalog, Page, Result, load_books};
use crate::model::{Book, BookPage, Editions};
use crate::rank::{RatingQuery, RatingSource};
use crate::search::BookAttrs;
use crate::text::edition_note;

/// Sizes above this do not rank higher.
pub const SIZE_CAP: u32 = 30 * 1024 * 1024;

/// One list row: a work with its editions in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The best copy.
    pub best: i64,
    /// All editions in the list, the best copy first, then in list order.
    pub members: Vec<i64>,
}

fn format_rank(ext: &str) -> u8 {
    match ext {
        "fb2" => 0,
        "epub" => 1,
        _ => 2,
    }
}

fn size_bucket(size: u32) -> u32 {
    let s = size.clamp(1, SIZE_CAP) as f64;
    (s.ln() / 1.2f64.ln()).floor() as u32
}

/// Orders two editions of one work: `Less` = `a` is the better copy (see the module docs).
pub fn edition_cmp(a: i64, b: i64, attrs: &BookAttrs, src: &dyn RatingSource) -> Ordering {
    let cover = |id: i64| match src.has_cover(id) {
        Some(true) => 0u8,
        None => 1,
        Some(false) => 2,
    };
    attrs
        .is_live(b)
        .cmp(&attrs.is_live(a))
        .then_with(|| attrs.names_volume(a).cmp(&attrs.names_volume(b)))
        .then_with(|| cover(a).cmp(&cover(b)))
        .then_with(|| format_rank(attrs.ext(a)).cmp(&format_rank(attrs.ext(b))))
        .then_with(|| size_bucket(attrs.size(b)).cmp(&size_bucket(attrs.size(a))))
        .then_with(|| attrs.date(b).cmp(&attrs.date(a)))
        .then_with(|| attrs.stars(b).cmp(&attrs.stars(a)))
        .then(a.cmp(&b))
}

/// Groups `ids` (in list order) by work: one [`Group`] per work at the position of its first
/// edition.
pub fn group_ids(ids: &[i64], attrs: &BookAttrs, src: &dyn RatingSource) -> Vec<Group> {
    let mut pos: HashMap<i64, usize> = HashMap::new();
    let mut groups: Vec<Group> = Vec::new();
    for &id in ids {
        let w = attrs.work(id);
        match pos.get(&w) {
            Some(&i) => groups[i].members.push(id),
            None => {
                pos.insert(w, groups.len());
                groups.push(Group {
                    best: id,
                    members: vec![id],
                });
            }
        }
    }
    for g in groups.iter_mut().filter(|g| g.members.len() > 1) {
        let best = *g
            .members
            .iter()
            .min_by(|a, b| edition_cmp(**a, **b, attrs, src))
            .expect("non-empty");
        g.best = best;
        g.members.retain(|&m| m != best);
        g.members.insert(0, best);
    }
    groups
}

/// With a rating sort, a work is placed by its best copy's rating (the row shows that copy),
/// stably, so rows stay ordered by what they display.
pub(crate) fn sort_groups(
    groups: &mut [Group],
    rq: &RatingQuery,
    attrs: &BookAttrs,
    src: &dyn RatingSource,
) {
    if rq.sort != crate::rank::RatingSort::None {
        groups.sort_by_key(|g| std::cmp::Reverse(rq.key(g.best, attrs, src)));
    }
}

/// Loads the best copy of each group, with `editions` set on multi-edition groups.
pub(crate) fn load_groups(conn: &rusqlite::Connection, groups: &[Group]) -> Result<Vec<Book>> {
    let ids: Vec<i64> = groups.iter().map(|g| g.best).collect();
    let mut books = load_books(conn, &ids)?;
    let by_best: HashMap<i64, &Group> = groups.iter().map(|g| (g.best, g)).collect();
    for b in &mut books {
        if let Some(g) = by_best.get(&b.id).filter(|g| g.members.len() > 1) {
            b.editions = Some(Editions {
                count: g.members.len(),
                ids: g.members.clone(),
            });
        }
    }
    Ok(books)
}

/// One edition of a work (`GET …/books/:id/editions`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edition {
    #[serde(flatten)]
    pub book: Book,
    /// What sets this edition apart, from its title (`другой перевод`, `пер. Н. Галь`) or
    /// translation keywords; `None` when nothing is known.
    pub note: Option<String>,
}

impl Catalog {
    /// [`books_rated`](Catalog::books_rated), optionally with the editions of one work grouped
    /// into one row (see the module docs). Grouped pages are cut from the whole grouped
    /// selection by offset; `total` counts rows (works).
    pub fn books_page(
        &self,
        sel: &BookSelector,
        filter: &BookFilter,
        rq: &RatingQuery,
        src: &dyn RatingSource,
        page: &Page,
        group: bool,
    ) -> Result<BookPage> {
        if !group {
            return self.books_rated(sel, filter, rq, src, page);
        }
        let offset: usize = match &page.cursor {
            None => 0,
            Some(c) if c.is_empty() => 0,
            Some(c) => c
                .parse()
                .map_err(|_| crate::catalog::CatalogError::BadCursor)?,
        };
        let limit = page.limit.max(1);
        let attrs = self.attrs()?;
        let mut ids = self.selection_ids(sel, filter, &attrs)?;
        rq.apply(&mut ids, &attrs, src);
        let mut groups = group_ids(&ids, &attrs, src);
        sort_groups(&mut groups, rq, &attrs, src);
        let total = groups.len();
        let end = (offset + limit).min(total);
        let slice = if offset < total {
            &groups[offset..end]
        } else {
            &[][..]
        };
        let conn = self.conn()?;
        Ok(BookPage {
            books: load_groups(&conn, slice)?,
            next_cursor: (end < total).then(|| end.to_string()),
            total: total as i64,
        })
    }

    /// Groups ids (in list order) by work — for callers that build their own lists (MCP).
    pub fn group_books(&self, ids: &[i64], src: &dyn RatingSource) -> Result<Vec<Group>> {
        let attrs = self.attrs()?;
        Ok(group_ids(ids, &attrs, src))
    }

    /// Loads the best copies of `groups` with their `editions`.
    pub fn load_grouped(&self, groups: &[Group]) -> Result<Vec<Book>> {
        let conn = self.conn()?;
        load_groups(&conn, groups)
    }

    /// All editions of the work of book `id`, the best copy first; deleted editions only when
    /// `include_deleted` (or when `id` itself is deleted). `None` for an unknown book.
    pub fn editions(
        &self,
        id: i64,
        include_deleted: bool,
        src: &dyn RatingSource,
    ) -> Result<Option<Vec<Edition>>> {
        let conn = self.conn()?;
        let row: Option<(i64, i64)> = conn
            .prepare_cached("SELECT work_id, deleted FROM book WHERE id=?1")?
            .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((work, deleted)) = row else {
            return Ok(None);
        };
        let include_deleted = include_deleted || deleted != 0;
        let mut ids: Vec<i64> = if work == 0 {
            vec![id]
        } else {
            conn.prepare_cached("SELECT id FROM book WHERE work_id=?1")?
                .query_map([work], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        let attrs = self.attrs()?;
        ids.retain(|&e| e == id || include_deleted || attrs.is_live(e));
        ids.sort_by(|a, b| edition_cmp(*a, *b, &attrs, src));
        let mut notes: HashMap<i64, Option<String>> = HashMap::new();
        {
            let mut st =
                conn.prepare_cached("SELECT id, title, keywords FROM book WHERE id IN rarray(?1)")?;
            let rows = st.query_map([crate::catalog::id_array(ids.iter().copied())], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for r in rows {
                let (bid, title, kw) = r?;
                notes.insert(bid, note_of(&title, &kw));
            }
        }
        Ok(Some(
            load_books(&conn, &ids)?
                .into_iter()
                .map(|b| Edition {
                    note: notes.remove(&b.id).flatten(),
                    book: b,
                })
                .collect(),
        ))
    }
}

/// Edition note from the title's trailing notes and translation keywords
/// (`перевод …`, `пер. …`, `translated by …`).
fn note_of(title: &str, keywords: &str) -> Option<String> {
    let mut parts: Vec<String> = edition_note(title).into_iter().collect();
    for k in keywords.split([',', ';']).map(str::trim) {
        let l = k.to_lowercase();
        if l.starts_with("пер") || l.contains("перевод") || l.contains("translat") {
            parts.push(k.to_string());
        }
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// A numbered book in a series, for the series-number rule ([`series_number_merges`]).
#[derive(Debug, Clone)]
pub struct SeriesBook {
    /// Work id from the title rule.
    pub work: i64,
    pub lang: String,
    pub series: i64,
    /// Series number, > 0.
    pub serno: i64,
    /// Author ids, the first author first.
    pub authors: Vec<i64>,
    /// [`crate::text::work_title_key_in_series`] of the title.
    pub title_key: String,
    /// [`crate::text::volume_number`] of the title.
    pub volume: Option<u64>,
}

/// A series number with more distinct titles than this marks a series with unreliable numbering.
pub const MAX_TITLES_PER_NUMBER: usize = 5;

/// Whether the numbering of one series (all its numbered books) can be trusted to identify
/// works: no number carries more than [`MAX_TITLES_PER_NUMBER`] distinct titles, not all
/// titles share one number (3 or more titles), and no single number holds most of the titles
/// of a series with 6 or more.
pub fn series_numbering_ok(books: &[&SeriesBook]) -> bool {
    let mut by_no: HashMap<i64, std::collections::HashSet<&str>> = HashMap::new();
    let mut all: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for b in books {
        by_no.entry(b.serno).or_default().insert(&b.title_key);
        all.insert(&b.title_key);
    }
    let max = by_no.values().map(|s| s.len()).max().unwrap_or(0);
    if max > MAX_TITLES_PER_NUMBER {
        return false;
    }
    if by_no.len() == 1 && all.len() >= 3 {
        return false;
    }
    !(all.len() >= 6 && max * 2 > all.len())
}

/// The series-number rule (see the module docs): returns `old work id → new work id` for the
/// works it joins (the smallest work id of each joined set wins); works it leaves alone are
/// absent.
pub fn series_number_merges(books: &[SeriesBook]) -> HashMap<i64, i64> {
    let mut by_series: HashMap<i64, Vec<&SeriesBook>> = HashMap::new();
    for b in books {
        by_series.entry(b.series).or_default().push(b);
    }
    let mut parent: HashMap<i64, i64> = HashMap::new();
    fn find(parent: &mut HashMap<i64, i64>, x: i64) -> i64 {
        let mut r = x;
        while let Some(&p) = parent.get(&r) {
            if p == r {
                break;
            }
            r = p;
        }
        let mut c = x;
        while c != r {
            let next = parent.get(&c).copied().unwrap_or(r);
            parent.insert(c, r);
            c = next;
        }
        r
    }
    fn union(parent: &mut HashMap<i64, i64>, a: i64, b: i64) {
        let (ra, rb) = (find(parent, a), find(parent, b));
        if ra != rb {
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent.insert(hi, lo);
            parent.entry(lo).or_insert(lo);
        }
    }
    let mut series_ids: Vec<i64> = by_series.keys().copied().collect();
    series_ids.sort_unstable();
    for sid in series_ids {
        let list = &by_series[&sid];
        if !series_numbering_ok(list) {
            continue;
        }
        // one bucket per language, number and first author
        let mut buckets: HashMap<(&str, i64, i64), Vec<&SeriesBook>> = HashMap::new();
        for b in list.iter().filter(|b| !b.authors.is_empty()) {
            buckets
                .entry((b.lang.as_str(), b.serno, b.authors[0]))
                .or_default()
                .push(b);
        }
        let mut keys: Vec<_> = buckets.keys().copied().collect();
        keys.sort_unstable();
        for k in keys {
            let bucket = &buckets[&k];
            let volumes: std::collections::HashSet<u64> =
                bucket.iter().filter_map(|b| b.volume).collect();
            // different volumes under one number: only the same volume joins
            let parts: Vec<Vec<&SeriesBook>> = if volumes.len() >= 2 {
                let mut vs: Vec<u64> = volumes.into_iter().collect();
                vs.sort_unstable();
                vs.iter()
                    .map(|v| {
                        bucket
                            .iter()
                            .filter(|b| b.volume == Some(*v))
                            .copied()
                            .collect()
                    })
                    .collect()
            } else {
                vec![bucket.clone()]
            };
            for mut part in parts {
                // author sets: a book joins a cluster whose set contains its own or is contained
                part.sort_by_key(|b| std::cmp::Reverse(b.authors.len()));
                let mut clusters: Vec<(Vec<i64>, i64)> = Vec::new();
                for b in part {
                    let mut set = b.authors.clone();
                    set.sort_unstable();
                    set.dedup();
                    let hit = clusters.iter().position(|(cs, _)| {
                        set.iter().all(|a| cs.binary_search(a).is_ok())
                            || cs.iter().all(|a| set.binary_search(a).is_ok())
                    });
                    match hit {
                        Some(i) => union(&mut parent, clusters[i].1, b.work),
                        None => {
                            parent.entry(b.work).or_insert(b.work);
                            clusters.push((set, b.work));
                        }
                    }
                }
            }
        }
    }
    let works: Vec<i64> = parent.keys().copied().collect();
    let mut out = HashMap::new();
    for w in works {
        let r = find(&mut parent, w);
        if r != w {
            out.insert(w, r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_and_notes() {
        assert!(size_bucket(2_000_000) > size_bucket(1_000_000));
        assert_eq!(size_bucket(40_000_000), size_bucket(SIZE_CAP));
        assert_eq!(size_bucket(1_000_000), size_bucket(1_010_000));
        assert_eq!(format_rank("fb2"), 0);
        assert!(format_rank("epub") < format_rank("pdf"));
        assert_eq!(
            note_of(
                "Солярис (другой перевод)",
                "фантастика, перевод Д. Брускина"
            ),
            Some("другой перевод; перевод Д. Брускина".into())
        );
        assert_eq!(note_of("Солярис", "фантастика"), None);
    }

    use crate::text::{volume_number, work_title_key_in_series};

    /// The importer's two rules over `(title, series, serno, authors)`: work id of each book
    /// (book ids are 1-based positions).
    fn works_of(books: &[(&str, i64, i64, &[i64])]) -> Vec<i64> {
        let mut by_key: HashMap<String, i64> = HashMap::new();
        let mut work = Vec::new();
        let mut cands = Vec::new();
        for (i, (title, series, serno, authors)) in books.iter().enumerate() {
            let id = i as i64 + 1;
            let serno = (*serno > 0).then_some(*serno);
            let key = work_title_key_in_series(title, serno);
            let w = match &key {
                Some(k) => {
                    let mut a = authors.to_vec();
                    a.sort_unstable();
                    *by_key.entry(format!("ru {k} {a:?}")).or_insert(id)
                }
                None => id,
            };
            work.push(w);
            if let (Some(k), Some(n)) = (key, serno) {
                cands.push(SeriesBook {
                    work: w,
                    lang: "ru".into(),
                    series: *series,
                    serno: n,
                    authors: authors.to_vec(),
                    title_key: k,
                    volume: volume_number(title),
                });
            }
        }
        let m = series_number_merges(&cands);
        work.iter().map(|w| *m.get(w).unwrap_or(w)).collect()
    }

    const ASIMOV: &[i64] = &[1];

    #[test]
    fn asimov_foundation_translations() {
        // Азимов Айзек, series «Академия [Азимов]» (series 7), as in a real Flibusta library
        let books: &[(&str, i64, i64, &[i64])] = &[
            ("Миры Айзека Азимова. Книга 5", 7, 1, ASIMOV),  // 0
            ("Прелюдия к Академии", 7, 1, ASIMOV),           // 1
            ("Прелюдия к Основанию", 7, 1, ASIMOV),          // 2
            ("Академия", 7, 3, ASIMOV),                      // 3
            ("Основание", 7, 3, ASIMOV),                     // 4
            ("Основание (другой перевод)", 7, 3, ASIMOV),    // 5
            ("Установление", 7, 3, ASIMOV),                  // 6
            ("Второй Фонд", 7, 5, ASIMOV),                   // 7
            ("Дублеры", 7, 5, ASIMOV),                       // 8
            ("Академия на краю гибели", 7, 6, ASIMOV),       // 9
            ("Академия на краю гибели", 7, 6, ASIMOV),       // 10
            ("Край Основания", 7, 6, ASIMOV),                // 11
            ("Миры Айзека Азимова. Книга 9", 7, 6, ASIMOV),  // 12
            ("Сообщество на краю", 7, 6, ASIMOV),            // 13
            ("Академия и Земля", 7, 7, ASIMOV),              // 14
            ("Миры Айзека Азимова. Книга 10", 7, 7, ASIMOV), // 15
            ("Основание и Земля", 7, 7, ASIMOV),             // 16
            ("Сообщество и Земля", 7, 7, ASIMOV),            // 17
            ("Страхи Академии", 7, 8, ASIMOV),               // 18
            ("Академия и Хаос", 7, 9, ASIMOV),               // 19
            ("Триумф Академии", 7, 10, ASIMOV),              // 20
            // unnumbered: never joined
            ("Академия. Вторая трилогия", 7, 0, &[1, 2, 3, 4]), // 21
            ("Академия. Книги 1-7", 7, 0, ASIMOV),              // 22
            ("Академия. Начало", 7, 0, ASIMOV),                 // 23
            ("Академия. Первая трилогия", 7, 0, ASIMOV),        // 24
            ("Миры Айзека Азимова. Книга 7", 7, 0, ASIMOV),     // 25
            ("Путь к Академии", 7, 0, ASIMOV),                  // 26
            ("Фонд", 7, 2, ASIMOV),                             // 27
            ("Фонд", 7, 2, ASIMOV),                             // 28
        ];
        let w = works_of(books);
        let same = |ix: &[usize]| ix.iter().all(|&i| w[i] == w[ix[0]]);
        assert!(same(&[0, 1, 2]), "#1");
        assert!(same(&[3, 4, 5, 6]), "#3");
        assert!(same(&[7, 8]), "#5");
        assert!(same(&[9, 10, 11, 12, 13]), "#6");
        assert!(same(&[14, 15, 16, 17]), "#7");
        assert!(same(&[27, 28]), "#2");
        // numbers stay apart
        let firsts = [0, 27, 3, 7, 9, 14, 18, 19, 20];
        for (a, &i) in firsts.iter().enumerate() {
            for &j in &firsts[a + 1..] {
                assert_ne!(w[i], w[j], "{} / {}", books[i].0, books[j].0);
            }
        }
        // unnumbered books are only their own work
        for i in 21..=26 {
            assert!(
                w.iter().enumerate().all(|(j, &x)| j == i || x != w[i]),
                "{}",
                books[i].0
            );
        }
    }

    #[test]
    fn volumes_never_join() {
        // Маринина, Каменская #37
        let m: &[i64] = &[5];
        let w = works_of(&[
            ("Люди за спиной. Том 1", 3, 37, m),
            ("Люди за спиной. Том 2", 3, 37, m),
            ("Люди за спиной, том 1", 3, 37, m),
            ("Люди за спиной. Том 1 [litres]", 3, 37, m),
            ("Люди за спиной. Том 2 (fb2)", 3, 37, m),
            ("Люди за спиной", 3, 37, m),
        ]);
        assert_eq!(w[0], w[2]);
        assert_eq!(w[0], w[3]);
        assert_eq!(w[1], w[4]);
        assert_ne!(w[0], w[1]);
        assert_ne!(w[5], w[0]);
        assert_ne!(w[5], w[1]);
        // outside a series, too
        let w = works_of(&[
            ("Люди за спиной. Том 1", 0, 0, m),
            ("Люди за спиной. Том 2", 0, 0, m),
        ]);
        assert_ne!(w[0], w[1]);
    }

    #[test]
    fn series_rule_guards() {
        let a: &[i64] = &[1];
        // other authors, other languages or conflicting co-authors do not join
        let w = works_of(&[
            ("Первый", 1, 1, a),
            ("Другой", 1, 1, &[2]),
            ("Третий", 1, 1, &[1, 3]),
            ("Четвертый", 1, 1, &[1, 4]),
            ("Пятый", 1, 2, a),
            ("Шестой", 1, 3, a),
            ("Седьмой", 1, 4, a),
            ("Восьмой", 1, 5, a),
            ("Девятый", 1, 6, a),
        ]);
        assert_ne!(w[0], w[1]);
        assert_eq!(w[0], w[2]); // {1} ⊂ {1,3}
        assert_ne!(w[2], w[3]); // {1,3} vs {1,4}
        // everything numbered #1: numbering says nothing
        let w = works_of(&[("Один", 2, 1, a), ("Два", 2, 1, a), ("Три", 2, 1, a)]);
        assert_ne!(w[0], w[1]);
        assert_ne!(w[1], w[2]);
        // a number with too many titles
        let many: Vec<(String, i64, i64, &[i64])> = (0..7)
            .map(|i| {
                (
                    format!("Книга про {}", ["а", "б", "в", "г", "д", "е", "ж"][i]),
                    3,
                    if i < 6 { 4 } else { 5 },
                    a,
                )
            })
            .collect();
        let refs: Vec<(&str, i64, i64, &[i64])> = many
            .iter()
            .map(|(t, s, n, a)| (t.as_str(), *s, *n, *a))
            .collect();
        let w = works_of(&refs);
        assert_ne!(w[0], w[1]);
    }
}
