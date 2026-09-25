//! Author summary (`GET /libraries/:lib/authors/:id/summary`) and co-authors
//! (`GET /libraries/:lib/authors/:id/coauthors`).
//!
//! Prolific authors on Flibusta appear in hundreds of anthologies, so "everybody who shares a
//! book with X" is thousands of names. Co-authors are therefore ranked by *direct* shared
//! books first (books with at most [`ANTHOLOGY_MIN_AUTHORS`]` - 1` authors, i.e. real
//! co-authorship like the Strugatsky brothers), then by all shared books.

use std::collections::HashMap;

use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::catalog::{Catalog, Result, id_array};

/// A book with at least this many authors counts as an anthology / collection.
pub const ANTHOLOGY_MIN_AUTHORS: usize = 4;

/// How many co-authors [`Catalog::author_summary`] returns.
const TOP_COAUTHORS: usize = 10;
/// How many genres [`Catalog::author_summary`] returns.
const TOP_GENRES: usize = 8;

/// A co-author of an author: `books` = shared live books, `direct` = those of them that are
/// not anthologies (fewer than [`ANTHOLOGY_MIN_AUTHORS`] authors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coauthor {
    pub id: i64,
    pub name: String,
    pub books: i64,
    pub direct: i64,
}

/// A series of the author with the number of the author's live books in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesCount {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

/// Overview of an author's live (non-deleted) books.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorSummary {
    pub id: i64,
    pub name: String,
    /// Live books.
    pub count: i64,
    /// Live books with at least [`ANTHOLOGY_MIN_AUTHORS`] authors.
    pub anthologies: i64,
    /// Series of the author's books, most books first (then by name).
    pub series: Vec<SeriesCount>,
    /// Live books outside any series.
    pub without_series: i64,
    /// `[(lang, count)]`, most frequent first.
    pub langs: Vec<(String, i64)>,
    /// `[(genre id, count)]` of the most frequent (assigned) genres, at most 8.
    pub genres: Vec<(u16, i64)>,
    /// Oldest and newest `date` of the live books (`YYYY-MM-DD`), empty when unknown.
    pub first_date: String,
    pub last_date: String,
    /// Top co-authors: at least one direct shared book or two shared books, ranked by
    /// `direct` desc, `books` desc, name; at most 10.
    pub coauthors: Vec<Coauthor>,
    /// All distinct co-authors (anyone sharing a live book).
    pub coauthor_count: i64,
}

struct AuthorBooks {
    /// (book id, series id, lang, date, number of authors)
    books: Vec<(i64, Option<i64>, String, String, usize)>,
}

impl Catalog {
    fn author_books(&self, id: i64) -> Result<AuthorBooks> {
        let conn = self.conn()?;
        let mut st = conn.prepare_cached(
            "SELECT b.id, b.series_id, b.lang, b.date, \
             (SELECT count(*) FROM book_author x WHERE x.book_id=b.id) \
             FROM book_author ba JOIN book b ON b.id=ba.book_id \
             WHERE ba.author_id=?1 AND b.deleted=0",
        )?;
        let books = st
            .query_map([id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get::<_, i64>(4)? as usize,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(AuthorBooks { books })
    }

    /// Everybody who shares a live book with author `id`, ranked by direct shared books,
    /// then all shared books, then name.
    pub fn coauthors(&self, id: i64) -> Result<Vec<Coauthor>> {
        let ab = self.author_books(id)?;
        self.coauthors_of(id, &ab)
    }

    fn coauthors_of(&self, id: i64, ab: &AuthorBooks) -> Result<Vec<Coauthor>> {
        let shared: Vec<i64> = ab.books.iter().filter(|b| b.4 > 1).map(|b| b.0).collect();
        if shared.is_empty() {
            return Ok(Vec::new());
        }
        let authors_of: HashMap<i64, usize> = ab.books.iter().map(|b| (b.0, b.4)).collect();
        let conn = self.conn()?;
        let mut st = conn.prepare_cached(
            "SELECT ba.book_id, a.id, a.name, a.sort_key FROM book_author ba \
             JOIN author a ON a.id=ba.author_id \
             WHERE ba.book_id IN rarray(?1) AND ba.author_id<>?2",
        )?;
        let mut acc: HashMap<i64, (Coauthor, String)> = HashMap::new();
        let mut q = st.query(rusqlite::params![id_array(shared), id])?;
        while let Some(r) = q.next()? {
            let book: i64 = r.get(0)?;
            let aid: i64 = r.get(1)?;
            let direct = authors_of.get(&book).copied().unwrap_or(0) < ANTHOLOGY_MIN_AUTHORS;
            let e = acc.entry(aid).or_insert_with(|| {
                (
                    Coauthor {
                        id: aid,
                        name: r.get(2).unwrap_or_default(),
                        books: 0,
                        direct: 0,
                    },
                    r.get(3).unwrap_or_default(),
                )
            });
            e.0.books += 1;
            if direct {
                e.0.direct += 1;
            }
        }
        let mut v: Vec<(Coauthor, String)> = acc.into_values().collect();
        v.sort_by(|a, b| {
            b.0.direct
                .cmp(&a.0.direct)
                .then(b.0.books.cmp(&a.0.books))
                .then(a.1.cmp(&b.1))
                .then(a.0.id.cmp(&b.0.id))
        });
        Ok(v.into_iter().map(|x| x.0).collect())
    }

    /// Overview of author `id` (`None` when unknown): counts, series, languages, genres,
    /// date range and top co-authors.
    pub fn author_summary(&self, id: i64) -> Result<Option<AuthorSummary>> {
        let Some(a) = self.author(id)? else {
            return Ok(None);
        };
        let ab = self.author_books(id)?;
        let mut s = AuthorSummary {
            id: a.id,
            name: a.name,
            count: ab.books.len() as i64,
            ..Default::default()
        };
        let mut series: HashMap<i64, i64> = HashMap::new();
        let mut langs: HashMap<&str, i64> = HashMap::new();
        for (_, sid, lang, date, n) in &ab.books {
            match sid {
                Some(sid) => *series.entry(*sid).or_default() += 1,
                None => s.without_series += 1,
            }
            *langs.entry(lang.as_str()).or_default() += 1;
            if *n >= ANTHOLOGY_MIN_AUTHORS {
                s.anthologies += 1;
            }
            if !date.is_empty() {
                if s.first_date.is_empty() || *date < s.first_date {
                    s.first_date = date.clone();
                }
                if *date > s.last_date {
                    s.last_date = date.clone();
                }
            }
        }
        let mut lv: Vec<(String, i64)> =
            langs.into_iter().map(|(l, c)| (l.to_string(), c)).collect();
        lv.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        s.langs = lv;

        let conn = self.conn()?;
        if !series.is_empty() {
            let mut st = conn
                .prepare_cached("SELECT id, name, sort_key FROM series WHERE id IN rarray(?1)")?;
            let mut named: Vec<(SeriesCount, String)> = st
                .query_map([id_array(series.keys().copied())], |r| {
                    let sid: i64 = r.get(0)?;
                    Ok((
                        SeriesCount {
                            id: sid,
                            name: r.get(1)?,
                            count: series.get(&sid).copied().unwrap_or(0),
                        },
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            named.sort_by(|a, b| b.0.count.cmp(&a.0.count).then(a.1.cmp(&b.1)));
            s.series = named.into_iter().map(|x| x.0).collect();
        }
        if !ab.books.is_empty() {
            let mut st = conn.prepare_cached(
                "SELECT genre_id, count(*) FROM book_genre WHERE book_id IN rarray(?1) \
                 GROUP BY genre_id ORDER BY count(*) DESC, genre_id LIMIT ?2",
            )?;
            s.genres = st
                .query_map(
                    rusqlite::params![id_array(ab.books.iter().map(|b| b.0)), TOP_GENRES as i64],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?
                .collect::<rusqlite::Result<_>>()?;
        }
        drop(conn);
        let all = self.coauthors_of(id, &ab)?;
        s.coauthor_count = all.len() as i64;
        s.coauthors = all
            .into_iter()
            .filter(|c| c.direct >= 1 || c.books >= 2)
            .take(TOP_COAUTHORS)
            .collect();
        Ok(Some(s))
    }

    /// Whether author `id` exists (cheap check for 404s).
    pub fn author_exists(&self, id: i64) -> Result<bool> {
        let conn = self.conn()?;
        let mut st = conn.prepare_cached("SELECT 1 FROM author WHERE id=?1")?;
        Ok(st.query_row([id], |_| Ok(())).optional()?.is_some())
    }
}
