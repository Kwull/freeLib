//! Rating filters and sorts for book lists and search: the user's own rating, the library
//! rating (INPX stars), an external rating (Open Library, supplied by the server) and the age
//! estimate (`kids`).
//!
//! Library rating and age come from [`BookAttrs`]; the user's and the external rating are looked
//! up per book id through a [`RatingSource`] the server builds per request (the user's ratings of
//! one library, and a dense per-catalog array of cached external ratings). Everything runs in
//! memory: candidate ids come from `BookAttrs` (genre, new arrivals) or one SQL query (author,
//! series, shelf), then filter + stable sort + one page of `load_books`.

use std::collections::HashSet;

use rusqlite::ToSql;

use crate::catalog::{BookFilter, BookSelector, Catalog, CountSel, Page, Result, load_books};
use crate::model::BookPage;
use crate::search::BookAttrs;

/// Ratings the catalog does not store.
pub trait RatingSource {
    /// The current user's rating of book `id`, 1..5; 0 = not rated.
    fn my(&self, id: i64) -> u8;
    /// External rating of book `id`: (average × 100, vote count); `None` = unknown or no votes.
    fn ext(&self, id: i64) -> Option<(u16, u32)>;
}

/// No user and no external ratings.
pub struct NoRatings;

impl RatingSource for NoRatings {
    fn my(&self, _: i64) -> u8 {
        0
    }
    fn ext(&self, _: i64) -> Option<(u16, u32)> {
        None
    }
}

/// Sort by a rating, best first; books without that rating last.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RatingSort {
    /// The list's own order (search: relevance).
    #[default]
    None,
    /// The user's rating.
    My,
    /// Library rating (INPX stars).
    Lib,
    /// External rating (average, then vote count).
    Ext,
}

impl RatingSort {
    /// `my`, `lib`, `ext` (API `sort=`); anything else is [`RatingSort::None`].
    pub fn parse(s: &str) -> RatingSort {
        match s {
            "my" | "myRating" => RatingSort::My,
            "lib" | "libRating" => RatingSort::Lib,
            "ext" | "extRating" => RatingSort::Ext,
            _ => RatingSort::None,
        }
    }
}

/// Rating filters and sort. `Default` = nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RatingQuery {
    pub sort: RatingSort,
    /// Minimum own rating 1..5 (0 = no filter).
    pub min_my: u8,
    /// Minimum library rating 1..5 (0 = no filter).
    pub min_lib: u8,
    /// Minimum external average × 100 (0 = no filter).
    pub min_ext: u16,
    /// Minimum external vote count (0 = no filter).
    pub min_ext_votes: u32,
    /// Only books the user has not rated.
    pub unrated_by_me: bool,
    /// Only books whose age estimate is known and at most this.
    pub kids_max_age: Option<u8>,
}

impl RatingQuery {
    /// Whether any filter or rating sort is set.
    pub fn is_active(&self) -> bool {
        self.sort != RatingSort::None || self.has_filter()
    }

    /// Whether any filter is set.
    pub fn has_filter(&self) -> bool {
        self.min_my > 0
            || self.min_lib > 0
            || self.min_ext > 0
            || self.min_ext_votes > 0
            || self.unrated_by_me
            || self.kids_max_age.is_some()
    }

    /// Whether book `id` passes the filters.
    pub fn accepts(&self, id: i64, attrs: &BookAttrs, src: &dyn RatingSource) -> bool {
        if self.min_lib > 0 && attrs.stars(id) < self.min_lib {
            return false;
        }
        if let Some(max) = self.kids_max_age {
            match attrs.age(id) {
                Some(a) if a <= max => {}
                _ => return false,
            }
        }
        if self.min_my > 0 || self.unrated_by_me {
            let my = src.my(id);
            if (self.min_my > 0 && my < self.min_my) || (self.unrated_by_me && my > 0) {
                return false;
            }
        }
        if self.min_ext > 0 || self.min_ext_votes > 0 {
            match src.ext(id) {
                Some((avg, votes)) if avg >= self.min_ext && votes >= self.min_ext_votes => {}
                _ => return false,
            }
        }
        true
    }

    /// Sort key (higher = better; 0 = unrated) of book `id`.
    pub fn key(&self, id: i64, attrs: &BookAttrs, src: &dyn RatingSource) -> u64 {
        match self.sort {
            RatingSort::None => 0,
            RatingSort::My => src.my(id) as u64,
            RatingSort::Lib => attrs.stars(id) as u64,
            RatingSort::Ext => match src.ext(id) {
                // average first, votes break ties
                Some((avg, votes)) if votes > 0 => ((avg as u64) << 32) | votes as u64,
                _ => 0,
            },
        }
    }

    /// Filters `ids` and, with a rating sort, sorts them stably: best first, unrated last, the
    /// incoming order among equals.
    pub fn apply(&self, ids: &mut Vec<i64>, attrs: &BookAttrs, src: &dyn RatingSource) {
        if self.has_filter() {
            ids.retain(|&id| self.accepts(id, attrs, src));
        }
        if self.sort != RatingSort::None {
            let mut keyed: Vec<(u64, i64)> = ids
                .iter()
                .map(|&id| (self.key(id, attrs, src), id))
                .collect();
            keyed.sort_by_key(|k| std::cmp::Reverse(k.0)); // stable
            *ids = keyed.into_iter().map(|(_, id)| id).collect();
        }
    }
}

impl Catalog {
    /// [`books`](Catalog::books) with rating filters and sorts. Without any (`!rq.is_active()`)
    /// this is exactly `books`. With them, the whole selection is filtered and sorted in memory
    /// and paged by offset; genre and new-arrival selections are ordered by date (newest first),
    /// then id, before a rating sort.
    pub fn books_rated(
        &self,
        sel: &BookSelector,
        filter: &BookFilter,
        rq: &RatingQuery,
        src: &dyn RatingSource,
        page: &Page,
    ) -> Result<BookPage> {
        if !rq.is_active() {
            return self.books(sel, filter, page);
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
        let total = ids.len() as i64;
        let end = (offset + limit).min(ids.len());
        let page_ids = if offset < ids.len() {
            &ids[offset..end]
        } else {
            &[][..]
        };
        let conn = self.conn()?;
        let books = load_books(&conn, page_ids)?;
        Ok(BookPage {
            books,
            next_cursor: (end < ids.len()).then(|| end.to_string()),
            total,
        })
    }

    /// All ids of a selection with the list filters, in list order (genre / since: date desc,
    /// id).
    pub fn selection_ids(
        &self,
        sel: &BookSelector,
        filter: &BookFilter,
        attrs: &BookAttrs,
    ) -> Result<Vec<i64>> {
        let conn = self.conn()?;
        let fts = filter.q.as_deref().and_then(crate::search::fts_query);
        let scan_sel = match sel {
            BookSelector::Genre(g) => Some(CountSel::Genres(
                crate::genres::genres().with_descendants(*g),
            )),
            BookSelector::Since(d) => Some(CountSel::Since(d.clone())),
            _ => None,
        };
        if let Some(cs) = scan_sel {
            let mut ids = attrs.scan(&cs, filter);
            if let Some(f) = &fts {
                let mut st =
                    conn.prepare_cached("SELECT rowid FROM book_fts WHERE book_fts MATCH ?1")?;
                let hits: HashSet<i64> = st
                    .query_map([f], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                ids.retain(|id| hits.contains(id));
            }
            ids.sort_by(|a, b| attrs.date(*b).cmp(&attrs.date(*a)).then(a.cmp(b)));
            return Ok(ids);
        }
        let q = self.selection_sql(&conn, sel, filter)?;
        let sql = format!(
            "SELECT b.id {}{} ORDER BY {}",
            q.from, q.where_extra, q.order
        );
        let mut st = conn.prepare_cached(&sql)?;
        let all: Vec<&dyn ToSql> = q.params.iter().map(|b| b.as_ref()).collect();
        Ok(st
            .query_map(all.as_slice(), |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl RatingSource for Fake {
        fn my(&self, id: i64) -> u8 {
            (id % 6) as u8
        }
        fn ext(&self, id: i64) -> Option<(u16, u32)> {
            (id % 3 != 0).then_some(((id * 37 % 500) as u16, (id * 11 % 50) as u32))
        }
    }

    #[test]
    fn parse_sort() {
        assert_eq!(RatingSort::parse("my"), RatingSort::My);
        assert_eq!(RatingSort::parse("extRating"), RatingSort::Ext);
        assert_eq!(RatingSort::parse("date"), RatingSort::None);
        assert!(!RatingQuery::default().is_active());
        let q = RatingQuery {
            kids_max_age: Some(6),
            ..Default::default()
        };
        assert!(q.is_active() && q.has_filter());
        let q = RatingQuery {
            sort: RatingSort::Lib,
            ..Default::default()
        };
        assert!(q.is_active() && !q.has_filter());
        let _ = Fake.my(1);
        let _ = Fake.ext(1);
    }
}
