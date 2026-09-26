//! The start page: "Continue series" and "New from authors I read" (the user's data — history,
//! ratings, shelves, follows — is supplied by the server as book ids and author/series ids).

use std::collections::{HashMap, HashSet};

use rusqlite::ToSql;
use serde::Serialize;

use crate::catalog::{BookFilter, BookSelector, Catalog, CountSel, Result, id_array};
use crate::model::{Book, SeriesHit};
use crate::rank::RatingSource;
use crate::summary::ANTHOLOGY_MIN_AUTHORS;
use crate::works::{group_ids, load_groups};

/// A series with more distinct first authors than this is a publisher series ("Мир
/// фантастики"): only books sharing an author with the user's books in it count.
pub const PUBLISHER_SERIES_AUTHORS: usize = 3;

/// A series the user has started, with the next unread book(s).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesNext {
    pub series: SeriesHit,
    /// Works in the series (for a publisher series: by the user's authors).
    pub works: usize,
    /// Works the user sent, downloaded, read or rated.
    pub done: usize,
    /// Latest activity in the series (RFC 3339), for ordering.
    pub last_at: String,
    /// The next works after the last one done that the user has not done, best copies.
    pub next: Vec<Book>,
}

/// Why a book is in "New from authors I read".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewReason {
    /// `"author"` or `"series"`.
    pub kind: &'static str,
    pub id: i64,
    pub name: String,
    /// Explicitly followed (else: an author the user read, rated ≥ 4 or shelved).
    pub followed: bool,
}

/// A book of "New from authors I read".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewBook {
    #[serde(flatten)]
    pub book: Book,
    pub reason: NewReason,
}

/// Seeds of "New from authors I read".
#[derive(Debug, Clone, Default)]
pub struct NewFromSeeds {
    /// Authors followed explicitly.
    pub followed_authors: Vec<i64>,
    /// Series followed explicitly.
    pub followed_series: Vec<i64>,
    /// Authors of books the user sent, downloaded, read, rated ≥ 4 or shelved (anthologies
    /// and unknown authors are skipped by [`Catalog::reading_authors`]).
    pub read_authors: Vec<i64>,
}

impl Catalog {
    /// Authors of `book_ids` worth following: books with fewer than
    /// [`ANTHOLOGY_MIN_AUTHORS`] authors, "Автор неизвестен" excluded.
    pub fn reading_authors(&self, book_ids: &[i64]) -> Result<Vec<i64>> {
        let conn = self.conn()?;
        let mut st = conn.prepare_cached(
            "SELECT ba.book_id, ba.author_id, a.name FROM book_author ba JOIN author a ON a.id=ba.author_id \
             WHERE ba.book_id IN rarray(?1)",
        )?;
        let mut per_book: HashMap<i64, Vec<(i64, String)>> = HashMap::new();
        for r in st.query_map([id_array(book_ids.iter().copied())], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (b, a, n) = r?;
            per_book.entry(b).or_default().push((a, n));
        }
        let mut out: Vec<i64> = Vec::new();
        let mut seen = HashSet::new();
        for authors in per_book.values() {
            if authors.len() >= ANTHOLOGY_MIN_AUTHORS {
                continue;
            }
            for (a, n) in authors {
                if n != "Автор неизвестен" && seen.insert(*a) {
                    out.push(*a);
                }
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    /// Series the user has started (`done`: book id → when, RFC 3339; sent / downloaded / read
    /// or rated), with the next works after the last one done, newest activity first. Series
    /// whose sort key is in `dismissed` and finished series (nothing left after the last work
    /// done) are left out. Editions of one work count as one.
    pub fn continue_series(
        &self,
        done: &HashMap<i64, String>,
        dismissed: &HashSet<String>,
        src: &dyn RatingSource,
        per_series: usize,
        limit: usize,
    ) -> Result<Vec<SeriesNext>> {
        if done.is_empty() {
            return Ok(Vec::new());
        }
        let attrs = self.attrs()?;
        let conn = self.conn()?;
        // series of the books done, with the latest activity and the authors
        let mut by_series: HashMap<i64, (String, HashSet<i64>)> = HashMap::new();
        {
            let ids: Vec<i64> = done.keys().copied().collect();
            let mut st = conn.prepare_cached(
                "SELECT b.id, b.series_id, ba.author_id FROM book b JOIN book_author ba ON ba.book_id=b.id \
                 WHERE b.id IN rarray(?1) AND b.series_id IS NOT NULL",
            )?;
            for r in st.query_map([id_array(ids)], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })? {
                let (b, s, a) = r?;
                let at = done.get(&b).cloned().unwrap_or_default();
                let e = by_series.entry(s).or_default();
                if at > e.0 {
                    e.0 = at;
                }
                e.1.insert(a);
            }
        }
        let mut order: Vec<(i64, String)> = by_series
            .iter()
            .map(|(s, (at, _))| (*s, at.clone()))
            .collect();
        order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut out = Vec::new();
        for (sid, last_at) in order {
            if out.len() >= limit {
                break;
            }
            let Some(series) = self.series(sid)? else {
                continue;
            };
            let key: String = conn
                .prepare_cached("SELECT sort_key FROM series WHERE id=?1")?
                .query_row([sid], |r| r.get(0))?;
            if dismissed.contains(&key) {
                continue;
            }
            let all = BookFilter {
                include_deleted: true,
                ..Default::default()
            };
            let mut ids = self.selection_ids(&BookSelector::Series(sid), &all, &attrs)?;
            ids.retain(|id| attrs.is_live(*id) || done.contains_key(id));
            let first_authors: i64 = conn
                .prepare_cached(
                    "SELECT count(DISTINCT first_author_id) FROM book WHERE series_id=?1",
                )?
                .query_row([sid], |r| r.get(0))?;
            if first_authors as usize > PUBLISHER_SERIES_AUTHORS {
                let authors = &by_series[&sid].1;
                let mut st = conn.prepare_cached(
                    "SELECT DISTINCT b.id FROM book b JOIN book_author ba ON ba.book_id=b.id \
                     WHERE b.series_id=?1 AND ba.author_id IN rarray(?2)",
                )?;
                let mine: HashSet<i64> = st
                    .query_map(
                        [
                            &sid as &dyn ToSql,
                            &id_array(authors.iter().copied()) as &dyn ToSql,
                        ],
                        |r| r.get(0),
                    )?
                    .collect::<rusqlite::Result<_>>()?;
                ids.retain(|id| mine.contains(id) || done.contains_key(id));
            }
            // works in series order; a work is done when any edition is
            let groups = group_ids(&ids, &attrs, src);
            let is_done: Vec<bool> = groups
                .iter()
                .map(|g| g.members.iter().any(|m| done.contains_key(m)))
                .collect();
            let Some(last) = is_done.iter().rposition(|d| *d) else {
                // only deleted editions were done
                continue;
            };
            let next: Vec<_> = groups
                .iter()
                .zip(&is_done)
                .skip(last + 1)
                .filter(|(_, d)| !**d)
                .map(|(g, _)| g.clone())
                .take(per_series)
                .collect();
            if next.is_empty() {
                continue; // finished
            }
            out.push(SeriesNext {
                series,
                works: groups.len(),
                done: is_done.iter().filter(|d| **d).count(),
                last_at,
                next: load_groups(&conn, &next)?,
            });
        }
        Ok(out)
    }

    /// Live books dated on or after `since` (`YYYY-MM-DD`) by the seed authors or in the seed
    /// series, one row per work, newest first, without works in `exclude` (book ids the user
    /// already has). Returns (up to `limit` books with the reason, total rows).
    pub fn new_from(
        &self,
        seeds: &NewFromSeeds,
        since: &str,
        exclude: &HashSet<i64>,
        src: &dyn RatingSource,
        limit: usize,
    ) -> Result<(Vec<NewBook>, usize)> {
        let attrs = self.attrs()?;
        let conn = self.conn()?;
        let mut authors: Vec<i64> = seeds
            .followed_authors
            .iter()
            .chain(&seeds.read_authors)
            .copied()
            .collect();
        authors.sort_unstable();
        authors.dedup();
        let mut ids: HashSet<i64> = HashSet::new();
        if !authors.is_empty() {
            let mut st = conn.prepare_cached(
                "SELECT b.id FROM book_author ba JOIN book b ON b.id=ba.book_id \
                 WHERE ba.author_id IN rarray(?1) AND b.date >= ?2 AND b.deleted=0",
            )?;
            for r in st.query_map(
                [
                    &id_array(authors.iter().copied()) as &dyn ToSql,
                    &since as &dyn ToSql,
                ],
                |r| r.get(0),
            )? {
                ids.insert(r?);
            }
        }
        if !seeds.followed_series.is_empty() {
            let mut st = conn.prepare_cached(
                "SELECT id FROM book WHERE series_id IN rarray(?1) AND date >= ?2 AND deleted=0",
            )?;
            for r in st.query_map(
                [
                    &id_array(seeds.followed_series.iter().copied()) as &dyn ToSql,
                    &since as &dyn ToSql,
                ],
                |r| r.get(0),
            )? {
                ids.insert(r?);
            }
        }
        let excluded_works: HashSet<i64> = exclude.iter().map(|id| attrs.work(*id)).collect();
        let mut ids: Vec<i64> = ids
            .into_iter()
            .filter(|id| !excluded_works.contains(&attrs.work(*id)))
            .collect();
        ids.sort_by(|a, b| attrs.date(*b).cmp(&attrs.date(*a)).then(a.cmp(b)));
        let mut groups = group_ids(&ids, &attrs, src);
        let total = groups.len();
        groups.truncate(limit);
        let books = load_groups(&conn, &groups)?;
        let fa: HashSet<i64> = seeds.followed_authors.iter().copied().collect();
        let fs: HashSet<i64> = seeds.followed_series.iter().copied().collect();
        let ra: HashSet<i64> = seeds.read_authors.iter().copied().collect();
        Ok((
            books
                .into_iter()
                .map(|b| {
                    let reason = b
                        .series
                        .as_ref()
                        .filter(|s| fs.contains(&s.id))
                        .map(|s| NewReason {
                            kind: "series",
                            id: s.id,
                            name: s.name.clone(),
                            followed: true,
                        })
                        .or_else(|| {
                            b.authors
                                .iter()
                                .find(|a| fa.contains(&a.id))
                                .map(|a| NewReason {
                                    kind: "author",
                                    id: a.id,
                                    name: a.name.clone(),
                                    followed: true,
                                })
                        })
                        .or_else(|| {
                            b.authors
                                .iter()
                                .find(|a| ra.contains(&a.id))
                                .map(|a| NewReason {
                                    kind: "author",
                                    id: a.id,
                                    name: a.name.clone(),
                                    followed: false,
                                })
                        })
                        .unwrap_or(NewReason {
                            kind: "author",
                            id: 0,
                            name: String::new(),
                            followed: false,
                        });
                    NewBook { book: b, reason }
                })
                .collect(),
            total,
        ))
    }

    /// Newest book date of the catalog (`YYYY-MM-DD`), `None` when empty.
    pub fn newest_date(&self) -> Result<Option<String>> {
        let conn = self.conn()?;
        Ok(conn
            .prepare_cached("SELECT max(date) FROM book WHERE deleted=0 AND date != ''")?
            .query_row([], |r| r.get(0))?)
    }

    /// Suggestions for a new user: the best-rated (library rating) works among the books of
    /// the `days` days before the newest book, newest first among equals.
    pub fn picks(&self, days: i64, src: &dyn RatingSource, limit: usize) -> Result<Vec<Book>> {
        let Some(newest) = self.newest_date()? else {
            return Ok(Vec::new());
        };
        let d = crate::search::date_num(&newest);
        let (y, m, dd) = ((d / 10000) as i64, d / 100 % 100, d % 100);
        let since_days = crate::util::days_from_civil(y, m, dd) - days;
        let (y, m, dd) = crate::util::civil_from_days(since_days);
        let since = format!("{y:04}-{m:02}-{dd:02}");
        let attrs = self.attrs()?;
        let mut ids = attrs.scan(&CountSel::Since(since), &BookFilter::default());
        ids.sort_by(|a, b| {
            attrs
                .stars(*b)
                .cmp(&attrs.stars(*a))
                .then(attrs.date(*b).cmp(&attrs.date(*a)))
                .then(a.cmp(b))
        });
        ids.truncate(limit * 4);
        let mut groups = group_ids(&ids, &attrs, src);
        groups.truncate(limit);
        let conn = self.conn()?;
        load_groups(&conn, &groups)
    }

    /// Resolves author sort keys (normalized names) to ids: `[(key, id)]` for keys present.
    pub fn authors_by_keys(&self, keys: &[String]) -> Result<Vec<(String, i64)>> {
        self.ids_by_sort_keys("author", keys)
    }

    /// Resolves series sort keys to ids.
    pub fn series_by_keys(&self, keys: &[String]) -> Result<Vec<(String, i64)>> {
        self.ids_by_sort_keys("series", keys)
    }

    fn ids_by_sort_keys(&self, table: &str, keys: &[String]) -> Result<Vec<(String, i64)>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn()?;
        let mut st = conn.prepare_cached(&format!(
            "SELECT sort_key, min(id) FROM {table} WHERE sort_key IN rarray(?1) GROUP BY sort_key"
        ))?;
        Ok(st
            .query_map([crate::catalog::text_array(keys)], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Sort key (normalized name) of an author, `None` when unknown.
    pub fn author_key(&self, id: i64) -> Result<Option<String>> {
        self.sort_key_of("author", id)
    }

    /// Sort key of a series, `None` when unknown.
    pub fn series_key(&self, id: i64) -> Result<Option<String>> {
        self.sort_key_of("series", id)
    }

    fn sort_key_of(&self, table: &str, id: i64) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        let conn = self.conn()?;
        Ok(conn
            .prepare_cached(&format!("SELECT sort_key FROM {table} WHERE id=?1"))?
            .query_row([id], |r| r.get(0))
            .optional()?)
    }
}
