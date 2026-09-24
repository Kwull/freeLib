//! Full-text search (FTS5 prefix per word) with filters and facets.
//!
//! FTS5 returns the matching book ids with a bm25 score; filtering and facet counting
//! then run in memory over [`BookAttrs`], a compact per-book attribute table
//! (≈ 12 bytes/book) loaded once per catalog. This keeps facet computation independent
//! of the number of SQL round trips.

use std::collections::HashMap;
use std::time::Instant;

use rusqlite::Connection;
use serde::Deserialize;

use crate::catalog::{load_books, BookFilter, Catalog, CountSel, Result};
use crate::genres::genres;
use crate::model::*;
use crate::normalize::search_tokens;

/// What to search for (`kind` query parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    #[default]
    All,
    Books,
    Authors,
    Series,
}

/// Search request (`GET /libraries/:lib/search`).
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub q: String,
    pub kind: SearchKind,
    /// Genre ids; a top-level group matches all its sub-genres. Empty = any.
    pub genres: Vec<u16>,
    /// Language codes. Empty = any.
    pub langs: Vec<String>,
    pub ext: Option<String>,
    /// Inclusive date range `YYYY-MM-DD`.
    pub from: Option<String>,
    pub to: Option<String>,
    pub include_deleted: bool,
    /// Max books returned (the API clamps to 1..=1000; default 200).
    pub limit: usize,
}

const FLAG_EXISTS: u8 = 1;
const FLAG_DELETED: u8 = 2;

/// Compact per-book attributes, indexed by book id.
pub struct BookAttrs {
    flags: Vec<u8>,
    lang: Vec<u16>,
    ext: Vec<u16>,
    /// `yyyymmdd` as an integer, 0 = unknown.
    date: Vec<u32>,
    genre_off: Vec<u32>,
    genre_ids: Vec<u16>,
    langs: Vec<String>,
    exts: Vec<String>,
}

fn date_num(s: &str) -> u32 {
    let d: String = s.chars().filter(|c| c.is_ascii_digit()).take(8).collect();
    if d.len() == 8 { d.parse().unwrap_or(0) } else { 0 }
}

impl BookAttrs {
    pub(crate) fn load(conn: &Connection) -> rusqlite::Result<BookAttrs> {
        let max_id: i64 = conn.query_row("SELECT coalesce(max(id), 0) FROM book", [], |r| r.get(0))?;
        let n = max_id as usize + 1;
        let mut a = BookAttrs {
            flags: vec![0; n],
            lang: vec![0; n],
            ext: vec![0; n],
            date: vec![0; n],
            genre_off: vec![0; n + 1],
            genre_ids: Vec::new(),
            langs: Vec::new(),
            exts: Vec::new(),
        };
        let mut lang_idx: HashMap<String, u16> = HashMap::new();
        let mut ext_idx: HashMap<String, u16> = HashMap::new();
        let mut st = conn.prepare("SELECT id, lang, ext, date, deleted FROM book")?;
        let mut q = st.query([])?;
        while let Some(r) = q.next()? {
            let id = r.get::<_, i64>(0)? as usize;
            let lang = r.get_ref(1)?.as_str().unwrap_or("");
            let li = match lang_idx.get(lang) {
                Some(&i) => i,
                None => {
                    let i = a.langs.len() as u16;
                    a.langs.push(lang.to_string());
                    lang_idx.insert(lang.to_string(), i);
                    i
                }
            };
            let ext = r.get_ref(2)?.as_str().unwrap_or("");
            let ei = match ext_idx.get(ext) {
                Some(&i) => i,
                None => {
                    let i = a.exts.len() as u16;
                    a.exts.push(ext.to_string());
                    ext_idx.insert(ext.to_string(), i);
                    i
                }
            };
            a.lang[id] = li;
            a.ext[id] = ei;
            a.date[id] = date_num(r.get_ref(3)?.as_str().unwrap_or(""));
            a.flags[id] = FLAG_EXISTS | if r.get::<_, i64>(4)? != 0 { FLAG_DELETED } else { 0 };
        }
        // Genres in CSR form; the reverse index yields rows ordered by book_id.
        let mut st = conn.prepare("SELECT book_id, genre_id FROM book_genre INDEXED BY book_bg_rev ORDER BY book_id")?;
        let mut q = st.query([])?;
        let mut counts = vec![0u32; n];
        let mut pairs: Vec<(u32, u16)> = Vec::new();
        while let Some(r) = q.next()? {
            let b = r.get::<_, i64>(0)? as usize;
            if b < n {
                counts[b] += 1;
                pairs.push((b as u32, r.get(1)?));
            }
        }
        let mut off = 0u32;
        for (i, c) in counts.iter().enumerate() {
            a.genre_off[i] = off;
            off += c;
        }
        a.genre_off[n] = off;
        a.genre_ids = pairs.into_iter().map(|(_, g)| g).collect();
        Ok(a)
    }

    fn genres_of(&self, id: usize) -> &[u16] {
        &self.genre_ids[self.genre_off[id] as usize..self.genre_off[id + 1] as usize]
    }

    /// Count books matching a genre set / date lower bound and the list filters.
    pub(crate) fn count(&self, sel: &CountSel, f: &BookFilter) -> i64 {
        let lang_ok: Vec<bool> = self.langs.iter().map(|l| f.langs.is_empty() || f.langs.iter().any(|x| x == l)).collect();
        let ext_ok: Vec<bool> =
            self.exts.iter().map(|e| f.ext.as_ref().is_none_or(|x| x.eq_ignore_ascii_case(e))).collect();
        let (mut gset, since) = match sel {
            CountSel::Genres(g) => (g.clone(), 0),
            CountSel::Since(d) => (Vec::new(), date_num(d).max(1)),
            CountSel::Newer(d) => (Vec::new(), date_num(d) + 1),
        };
        gset.sort_unstable();
        let mut n = 0i64;
        for i in 0..self.flags.len() {
            let fl = self.flags[i];
            if fl & FLAG_EXISTS == 0 || (!f.include_deleted && fl & FLAG_DELETED != 0) {
                continue;
            }
            if !lang_ok[self.lang[i] as usize] || !ext_ok[self.ext[i] as usize] || self.date[i] < since {
                continue;
            }
            if !gset.is_empty() && !self.genres_of(i).iter().any(|g| gset.binary_search(g).is_ok()) {
                continue;
            }
            n += 1;
        }
        n
    }

    /// Approximate heap size in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.flags.len() * (1 + 2 + 2 + 4 + 4) + self.genre_ids.len() * 2
    }
}

/// Build the FTS5 MATCH expression: every token as a quoted prefix, implicitly AND-ed.
/// One-letter tokens are dropped when longer ones exist (`Война и мир` → `война* мир*`),
/// since `и*` would match most of the catalog without narrowing the result.
pub fn fts_query(q: &str) -> Option<String> {
    let tokens = search_tokens(q);
    let long: Vec<&String> = tokens.iter().filter(|t| t.chars().count() >= 2).collect();
    let used: Vec<&String> = if long.is_empty() { tokens.iter().collect() } else { long };
    if used.is_empty() {
        return None;
    }
    Some(used.iter().map(|t| format!("\"{t}\"*")).collect::<Vec<_>>().join(" "))
}

impl Catalog {
    /// Full-text search over titles, authors, series and keywords, plus author and series name
    /// matches. `q` must have at least 2 characters, otherwise the result is empty.
    pub fn search(&self, sq: &SearchQuery) -> Result<SearchResult> {
        let started = Instant::now();
        let mut res = SearchResult::default();
        let fts = match fts_query(&sq.q) {
            Some(f) if sq.q.trim().chars().count() >= 2 => f,
            _ => return Ok(res),
        };
        let conn = self.conn()?;

        if matches!(sq.kind, SearchKind::All | SearchKind::Authors) {
            let mut st = conn.prepare_cached(
                "SELECT a.id, a.name, a.book_count FROM author a \
                 WHERE a.id IN (SELECT rowid FROM author_fts WHERE author_fts MATCH ?1) \
                 ORDER BY a.book_count DESC, a.sort_key LIMIT 20",
            )?;
            res.authors = st
                .query_map([&fts], |r| Ok(NameCount { id: r.get(0)?, name: r.get(1)?, count: r.get(2)? }))?
                .collect::<rusqlite::Result<_>>()?;
        }
        if matches!(sq.kind, SearchKind::All | SearchKind::Series) {
            let mut st = conn.prepare_cached(
                "SELECT s.id, s.name, s.book_count, s.authors FROM series s \
                 WHERE s.id IN (SELECT rowid FROM series_fts WHERE series_fts MATCH ?1) \
                 ORDER BY s.book_count DESC, s.sort_key LIMIT 20",
            )?;
            res.series = st
                .query_map([&fts], |r| {
                    Ok(SeriesHit { id: r.get(0)?, name: r.get(1)?, count: r.get(2)?, authors: r.get(3)? })
                })?
                .collect::<rusqlite::Result<_>>()?;
        }
        if matches!(sq.kind, SearchKind::All | SearchKind::Books) {
            let attrs = self.attrs()?;
            let hits: Vec<(i64, f64)> = {
                // Column weights: title, authors, series, keywords.
                let mut st = conn.prepare_cached(
                    "SELECT rowid, bm25(book_fts, 10.0, 4.0, 3.0, 1.0) FROM book_fts WHERE book_fts MATCH ?1",
                )?;
                st.query_map([&fts], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
            };
            let (ranked, total, facets) = filter_and_facet(&attrs, &hits, sq);
            let limit = sq.limit.clamp(1, 1000);
            let top = top_n(ranked, limit, &attrs);
            res.books = load_books(&conn, &top)?;
            res.total = total;
            res.facets = facets;
        }
        res.took_ms = started.elapsed().as_millis() as u64;
        Ok(res)
    }
}

/// Apply filters; count disjunctive facets (each facet ignores its own filter).
fn filter_and_facet(a: &BookAttrs, hits: &[(i64, f64)], sq: &SearchQuery) -> (Vec<(i64, f64)>, i64, Facets) {
    let gset: Vec<u16> = {
        let mut v: Vec<u16> = sq.genres.iter().flat_map(|&g| genres().with_descendants(g)).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let lang_ok: Vec<bool> = a.langs.iter().map(|l| sq.langs.is_empty() || sq.langs.iter().any(|x| x == l)).collect();
    let ext_ok: Vec<bool> =
        a.exts.iter().map(|e| sq.ext.as_ref().is_none_or(|x| x.eq_ignore_ascii_case(e))).collect();
    let from = sq.from.as_deref().map(date_num).unwrap_or(0);
    let to = sq.to.as_deref().map(date_num).filter(|&d| d > 0).unwrap_or(u32::MAX);

    let mut out = Vec::new();
    let mut g_counts: HashMap<u16, i64> = HashMap::new();
    let mut l_counts = vec![0i64; a.langs.len()];
    let mut e_counts = vec![0i64; a.exts.len()];
    for &(id, score) in hits {
        let i = id as usize;
        if i >= a.flags.len() || a.flags[i] & FLAG_EXISTS == 0 {
            continue;
        }
        if !sq.include_deleted && a.flags[i] & FLAG_DELETED != 0 {
            continue;
        }
        let d = a.date[i];
        if d < from || d > to {
            continue;
        }
        let bg = a.genres_of(i);
        let g_ok = gset.is_empty() || bg.iter().any(|g| gset.binary_search(g).is_ok());
        let l_ok = lang_ok[a.lang[i] as usize];
        let e_ok = ext_ok[a.ext[i] as usize];
        if l_ok && e_ok {
            for g in bg {
                *g_counts.entry(*g).or_default() += 1;
            }
        }
        if g_ok && e_ok {
            l_counts[a.lang[i] as usize] += 1;
        }
        if g_ok && l_ok {
            e_counts[a.ext[i] as usize] += 1;
        }
        if g_ok && l_ok && e_ok {
            out.push((id, score));
        }
    }
    let total = out.len() as i64;
    let mut genre: Vec<(u16, i64)> = g_counts.into_iter().collect();
    genre.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    let mut lang: Vec<(String, i64)> =
        l_counts.iter().enumerate().filter(|(_, c)| **c > 0).map(|(i, c)| (a.langs[i].clone(), *c)).collect();
    lang.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    let mut ext: Vec<(String, i64)> =
        e_counts.iter().enumerate().filter(|(_, c)| **c > 0).map(|(i, c)| (a.exts[i].clone(), *c)).collect();
    ext.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    (out, total, Facets { genre, lang, ext })
}

/// Best `n` hits: bm25 ascending (more relevant first), then newer first, then id.
fn top_n(mut v: Vec<(i64, f64)>, n: usize, a: &BookAttrs) -> Vec<i64> {
    let cmp = |x: &(i64, f64), y: &(i64, f64)| {
        x.1.total_cmp(&y.1).then_with(|| a.date[y.0 as usize].cmp(&a.date[x.0 as usize])).then(x.0.cmp(&y.0))
    };
    if v.len() > n {
        v.select_nth_unstable_by(n, cmp);
        v.truncate(n);
    }
    v.sort_by(cmp);
    v.into_iter().map(|x| x.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_expr() {
        assert_eq!(fts_query("Война и мир").unwrap(), "\"война\"* \"мир\"*");
        assert_eq!(fts_query("а б").unwrap(), "\"а\"* \"б\"*");
        assert_eq!(fts_query("Ёжик\"*)").unwrap(), "\"ежик\"*");
        assert!(fts_query(" ,. ").is_none());
    }

    #[test]
    fn dates() {
        assert_eq!(date_num("2024-01-31"), 20240131);
        assert_eq!(date_num(""), 0);
        assert_eq!(date_num("2024"), 0);
    }
}
