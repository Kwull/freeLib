//! Editions of one work shown as one row, with the best copy picked automatically.
//!
//! The importer gives every book a `work_id`: the id of the first book with the same language,
//! title key ([`crate::text::work_title_key`]: normalized title without trailing edition notes
//! such as `(другой перевод)`) and author set. Books by an unknown author and generic titles
//! ("Избранное", "Рассказы") keep their own id. A list is grouped in memory: each work appears
//! once, at the position of its first edition in the list, represented by its best copy.
//!
//! **Best copy** (first difference wins):
//! 1. not deleted;
//! 2. has a cover (as far as the server knows — covers are only known for books whose preview
//!    was extracted; unknown ranks between yes and no);
//! 3. format: FB2 (converts best, usually with its cover), then EPUB, then anything else;
//! 4. larger file, compared in 20 % steps and capped at 30 MB (a bigger file usually has the
//!    illustrations and the complete text; beyond the cap size says nothing);
//! 5. newer date (a later upload is usually a corrected version);
//! 6. higher library rating;
//! 7. lower id (stable).

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
}
