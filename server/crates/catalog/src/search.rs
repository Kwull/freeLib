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

use crate::catalog::{BookFilter, Catalog, CountSel, Result, load_books};
use crate::genres::genres;
use crate::model::*;
use crate::normalize::search_tokens;
use crate::text::{latin_key, stem};

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
    /// Rating filters (applied before facets are counted) and an optional rating sort
    /// (instead of relevance; relevance breaks ties).
    pub rating: crate::rank::RatingQuery,
    /// One row per work (editions grouped, see [`crate::works`]); `total` then counts works.
    pub group: bool,
    /// Search the query exactly as typed: no typo correction ("Search instead for …").
    pub exact: bool,
}

pub(crate) const FLAG_EXISTS: u8 = 1;
pub(crate) const FLAG_DELETED: u8 = 2;
/// The title names a volume (`Книга 9`, `Том 1`): an omnibus or a part, not the best copy of
/// a work that also has plain copies.
pub(crate) const FLAG_VOLUME: u8 = 4;

/// Compact per-book attributes, indexed by book id.
pub struct BookAttrs {
    pub(crate) flags: Vec<u8>,
    pub(crate) lang: Vec<u16>,
    pub(crate) ext: Vec<u16>,
    /// `yyyymmdd` as an integer, 0 = unknown.
    pub(crate) date: Vec<u32>,
    /// INPX stars (library rating) 0..5.
    pub(crate) stars: Vec<u8>,
    /// Age estimate (`kids::age_code`), `kids::AGE_UNKNOWN` = unknown.
    pub(crate) age: Vec<u8>,
    /// File size in bytes (clamped to `u32`).
    pub(crate) size: Vec<u32>,
    /// Work id (`book.work_id`: the first book of the work), 0 = none.
    pub(crate) work: Vec<u32>,
    pub(crate) genre_off: Vec<u32>,
    pub(crate) genre_ids: Vec<u16>,
    pub(crate) langs: Vec<String>,
    pub(crate) exts: Vec<String>,
}

pub(crate) fn date_num(s: &str) -> u32 {
    let d: String = s.chars().filter(|c| c.is_ascii_digit()).take(8).collect();
    if d.len() == 8 {
        d.parse().unwrap_or(0)
    } else {
        0
    }
}

impl BookAttrs {
    pub(crate) fn load(conn: &Connection) -> rusqlite::Result<BookAttrs> {
        let max_id: i64 =
            conn.query_row("SELECT coalesce(max(id), 0) FROM book", [], |r| r.get(0))?;
        let n = max_id as usize + 1;
        let mut a = BookAttrs {
            flags: vec![0; n],
            lang: vec![0; n],
            ext: vec![0; n],
            date: vec![0; n],
            stars: vec![0; n],
            age: vec![crate::kids::AGE_UNKNOWN; n],
            size: vec![0; n],
            work: vec![0; n],
            genre_off: vec![0; n + 1],
            genre_ids: Vec::new(),
            langs: Vec::new(),
            exts: Vec::new(),
        };
        let mut lang_idx: HashMap<String, u16> = HashMap::new();
        let mut ext_idx: HashMap<String, u16> = HashMap::new();
        // keywords only feed the age estimate; most books have none
        let mut keywords: Vec<(u32, String)> = Vec::new();
        let mut st = conn.prepare(
            "SELECT id, lang, ext, date, deleted, stars, keywords, size, work_id, title FROM book",
        )?;
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
            a.stars[id] = r.get::<_, i64>(5)?.clamp(0, 5) as u8;
            a.size[id] = r.get::<_, i64>(7)?.clamp(0, u32::MAX as i64) as u32;
            a.work[id] = r.get::<_, i64>(8)?.clamp(0, u32::MAX as i64) as u32;
            let kw = r.get_ref(6)?.as_str().unwrap_or("");
            if !kw.is_empty() {
                keywords.push((id as u32, kw.to_string()));
            }
            let title = r.get_ref(9)?.as_str().unwrap_or("");
            let volume = title.bytes().any(|b| b.is_ascii_digit())
                && crate::text::volume_number(title).is_some();
            a.flags[id] = FLAG_EXISTS
                | if r.get::<_, i64>(4)? != 0 {
                    FLAG_DELETED
                } else {
                    0
                }
                | if volume { FLAG_VOLUME } else { 0 };
        }
        // Genres in CSR form; the reverse index yields rows ordered by book_id.
        let mut st = conn.prepare(
            "SELECT book_id, genre_id FROM book_genre INDEXED BY book_bg_rev ORDER BY book_id",
        )?;
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
        let mut kw_iter = keywords.into_iter().peekable();
        for i in 0..n {
            if a.flags[i] & FLAG_EXISTS == 0 {
                continue;
            }
            let kw = match kw_iter.peek() {
                Some((k, _)) if *k as usize == i => kw_iter.next().map(|x| x.1),
                _ => None,
            };
            let g = &a.genre_ids[a.genre_off[i] as usize..a.genre_off[i + 1] as usize];
            if !g.is_empty() || kw.is_some() {
                a.age[i] = crate::kids::age_code(g, kw.as_deref().unwrap_or(""));
            }
        }
        Ok(a)
    }

    /// Library rating (INPX stars) of book `id`.
    pub fn stars(&self, id: i64) -> u8 {
        self.stars.get(id as usize).copied().unwrap_or(0)
    }

    /// Age estimate of book `id` (`None` = unknown).
    pub fn age(&self, id: i64) -> Option<u8> {
        self.age
            .get(id as usize)
            .copied()
            .filter(|a| *a != crate::kids::AGE_UNKNOWN)
    }

    /// Genre ids of book `id`.
    pub fn genres(&self, id: i64) -> &[u16] {
        let i = id as usize;
        if i + 1 >= self.genre_off.len() {
            return &[];
        }
        self.genres_of(i)
    }

    /// `yyyymmdd` of book `id`, 0 = unknown.
    pub fn date(&self, id: i64) -> u32 {
        self.date.get(id as usize).copied().unwrap_or(0)
    }

    /// Language code of book `id`.
    pub fn lang(&self, id: i64) -> &str {
        self.lang
            .get(id as usize)
            .and_then(|l| self.langs.get(*l as usize))
            .map(String::as_str)
            .unwrap_or("")
    }

    /// File extension of book `id`.
    pub fn ext(&self, id: i64) -> &str {
        self.ext
            .get(id as usize)
            .and_then(|e| self.exts.get(*e as usize))
            .map(String::as_str)
            .unwrap_or("")
    }

    /// File size of book `id` in bytes.
    pub fn size(&self, id: i64) -> u32 {
        self.size.get(id as usize).copied().unwrap_or(0)
    }

    /// Work id of book `id` (the id of the work's first book; the book's own id when it is
    /// not grouped with others).
    pub fn work(&self, id: i64) -> i64 {
        match self.work.get(id as usize).copied().unwrap_or(0) {
            0 => id,
            w => w as i64,
        }
    }

    /// Whether the title of book `id` names a volume ([`crate::text::volume_number`]).
    pub fn names_volume(&self, id: i64) -> bool {
        self.flags
            .get(id as usize)
            .is_some_and(|f| f & FLAG_VOLUME != 0)
    }

    /// Whether book `id` exists and is live (not deleted).
    pub fn is_live(&self, id: i64) -> bool {
        self.flags
            .get(id as usize)
            .is_some_and(|f| f & FLAG_EXISTS != 0 && f & FLAG_DELETED == 0)
    }

    /// Number of id slots (max id + 1).
    pub fn len(&self) -> usize {
        self.flags.len()
    }

    /// Whether the catalog has no books.
    pub fn is_empty(&self) -> bool {
        self.flags.len() <= 1
    }

    /// Ids of the books matching a genre set / date bound and the list filters (except the text
    /// filter), in id order.
    pub(crate) fn scan(&self, sel: &CountSel, f: &BookFilter) -> Vec<i64> {
        let mut out = Vec::new();
        self.for_each_match(sel, f, |i| out.push(i as i64));
        out
    }

    fn genres_of(&self, id: usize) -> &[u16] {
        &self.genre_ids[self.genre_off[id] as usize..self.genre_off[id + 1] as usize]
    }

    /// Count books matching a genre set / date lower bound and the list filters.
    pub(crate) fn count(&self, sel: &CountSel, f: &BookFilter) -> i64 {
        let mut n = 0i64;
        self.for_each_match(sel, f, |_| n += 1);
        n
    }

    fn for_each_match(&self, sel: &CountSel, f: &BookFilter, mut hit: impl FnMut(usize)) {
        let lang_ok: Vec<bool> = self
            .langs
            .iter()
            .map(|l| f.langs.is_empty() || f.langs.iter().any(|x| x == l))
            .collect();
        let ext_ok: Vec<bool> = self
            .exts
            .iter()
            .map(|e| f.ext.as_ref().is_none_or(|x| x.eq_ignore_ascii_case(e)))
            .collect();
        let (mut gset, since) = match sel {
            CountSel::Genres(g) => (g.clone(), 0),
            CountSel::Since(d) => (Vec::new(), date_num(d).max(1)),
            CountSel::Newer(d) => (Vec::new(), date_num(d) + 1),
        };
        gset.sort_unstable();
        for i in 0..self.flags.len() {
            let fl = self.flags[i];
            if fl & FLAG_EXISTS == 0 || (!f.include_deleted && fl & FLAG_DELETED != 0) {
                continue;
            }
            if !lang_ok[self.lang[i] as usize]
                || !ext_ok[self.ext[i] as usize]
                || self.date[i] < since
            {
                continue;
            }
            if !gset.is_empty()
                && !self
                    .genres_of(i)
                    .iter()
                    .any(|g| gset.binary_search(g).is_ok())
            {
                continue;
            }
            hit(i);
        }
    }

    /// Approximate heap size in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.flags.len() * (1 + 2 + 2 + 4 + 1 + 1 + 4 + 4 + 4) + self.genre_ids.len() * 2
    }
}

/// Words of `q` used for matching: one-letter words are dropped when longer ones exist
/// (`Война и мир` → `война`, `мир`), since `и*` would match most of the catalog.
fn used_tokens(all: &[String]) -> Vec<String> {
    let long: Vec<String> = all
        .iter()
        .filter(|t| t.chars().count() >= 2)
        .cloned()
        .collect();
    if long.is_empty() { all.to_vec() } else { long }
}

/// Build the FTS5 MATCH expression: every token as a quoted prefix, implicitly AND-ed.
/// One-letter tokens are dropped when longer ones exist (`Война и мир` → `война* мир*`),
/// since `и*` would match most of the catalog without narrowing the result.
pub fn fts_query(q: &str) -> Option<String> {
    let used = used_tokens(&search_tokens(q));
    if used.is_empty() {
        return None;
    }
    Some(
        used.iter()
            .map(|t| format!("\"{t}\"*"))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Ways a query word can match besides its own prefix: the same stem (word form: `книгу` →
/// `книг`, matched exactly against words and stored stems) and the same Latin key
/// (transliteration: `strugatsky` ↔ `стругацкий`; a prefix from 4 characters, exact for 3).
fn alternatives(t: &str) -> Vec<String> {
    let mut v = vec![format!("\"{t}\"*")];
    let n = t.chars().count();
    if n >= 3 {
        let s = stem(t);
        if s != t {
            v.push(format!("\"{s}\""));
        }
        if let Some(k) = latin_key(t)
            && k != t
        {
            v.push(if n >= 4 {
                format!("\"{k}\"*")
            } else {
                format!("\"{k}\"")
            });
        }
    }
    v
}

/// Relevance tiers are added to bm25 (lower = better) in steps of this size.
const TIER: f64 = 1000.0;

/// Fewer matches than this (books + authors + series) try a typo correction.
const FEW_RESULTS: i64 = 3;

/// How one query is matched. Tiers, best first: 3 = the title starts with the query word /
/// contains the query as a phrase (authors, series: the name starts with it); 2 = every word
/// is a prefix of a word; 1 = some words matched only by word form or transliteration
/// (`broad`); 0 = found through a typo correction.
pub(crate) struct Plan {
    pub(crate) tokens: Vec<String>,
    pub(crate) strict: String,
    pub(crate) broad: Option<String>,
    pub(crate) phrase: String,
    pub(crate) phrase_key: String,
}

pub(crate) fn plan(q: &str) -> Option<Plan> {
    let all = search_tokens(q);
    let tokens = used_tokens(&all);
    if tokens.is_empty() {
        return None;
    }
    let strict = tokens
        .iter()
        .map(|t| format!("\"{t}\"*"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut expanded = false;
    let parts: Vec<String> = tokens
        .iter()
        .map(|t| {
            let alts = alternatives(t);
            if alts.len() > 1 {
                expanded = true;
                format!("({})", alts.join(" OR "))
            } else {
                alts[0].clone()
            }
        })
        .collect();
    let phrase = if all.len() >= 2 {
        format!("title : \"{}\"*", all.join(" "))
    } else {
        format!("title : ^ \"{}\"*", all[0])
    };
    Some(Plan {
        strict,
        broad: expanded.then(|| parts.join(" AND ")),
        phrase,
        phrase_key: all.join(" "),
        tokens,
    })
}

/// Words of displayed names that a query word matched: by prefix, by stem, or by Latin key.
struct Highlighter {
    tokens: Vec<(String, String, Option<String>)>,
    seen: HashMap<String, bool>,
}

impl Highlighter {
    fn new(tokens: &[String]) -> Highlighter {
        Highlighter {
            tokens: tokens
                .iter()
                .map(|t| {
                    let n = t.chars().count();
                    let s = if n >= 3 { stem(t) } else { String::new() };
                    (t.clone(), s, latin_key(t))
                })
                .collect(),
            seen: HashMap::new(),
        }
    }

    fn add(&mut self, text: &str) {
        for w in search_tokens(text) {
            if self.seen.contains_key(&w) {
                continue;
            }
            let hit = self.tokens.iter().any(|(t, s, k)| {
                if w.starts_with(t.as_str()) {
                    return true;
                }
                if !s.is_empty() && stem(&w) == *s {
                    return true;
                }
                match (k, latin_key(&w)) {
                    (Some(k), Some(wk)) if t.chars().count() >= 4 => wk.starts_with(k.as_str()),
                    (Some(k), Some(wk)) => wk == *k,
                    _ => false,
                }
            });
            self.seen.insert(w, hit);
        }
    }

    fn words(self) -> Vec<String> {
        let mut v: Vec<String> = self
            .seen
            .into_iter()
            .filter(|(_, hit)| *hit)
            .map(|(w, _)| w)
            .collect();
        v.sort();
        v
    }
}

fn highlight(tokens: &[String], res: &SearchResult) -> Vec<String> {
    let mut h = Highlighter::new(tokens);
    for a in &res.authors {
        h.add(&a.name);
    }
    for s in &res.series {
        h.add(&s.name);
    }
    for b in &res.books {
        h.add(&b.title);
        for a in &b.authors {
            h.add(&a.name);
        }
        if let Some(s) = &b.series {
            h.add(&s.name);
        }
    }
    h.words()
}

impl Catalog {
    /// Full-text search over titles, authors, series and keywords, plus author and series name
    /// matches. `q` must have at least 2 characters, otherwise the result is empty.
    pub fn search(&self, sq: &SearchQuery) -> Result<SearchResult> {
        self.search_rated(sq, &crate::rank::NoRatings)
    }

    /// [`search`](Self::search) with the user's and external ratings for `sq.rating`.
    ///
    /// Words match as prefixes, word forms (Snowball stems) and transliterations (Latin keys);
    /// results are ranked by tier (see [`Plan`]), then bm25. When the query finds fewer than 3
    /// matches or no author/series, words unknown to the catalog are corrected by edit distance:
    /// when the corrected query finds more (or finds authors/series the query did not) its
    /// results are returned (`corrected`), else it is offered (`did_you_mean`). `exact` skips
    /// the correction.
    pub fn search_rated(
        &self,
        sq: &SearchQuery,
        src: &dyn crate::rank::RatingSource,
    ) -> Result<SearchResult> {
        let started = Instant::now();
        if sq.q.trim().chars().count() < 2 {
            return Ok(SearchResult::default());
        }
        let Some(p) = plan(&sq.q) else {
            return Ok(SearchResult::default());
        };
        let conn = self.conn()?;
        let mut res = self.search_plan(&conn, &p, sq, src)?;
        let mut used = p.tokens.clone();
        let found = |r: &SearchResult| r.total + r.authors.len() as i64 + r.series.len() as i64;
        let n = found(&res);
        let names = |r: &SearchResult| r.authors.len() + r.series.len();
        let wants_names = sq.kind != SearchKind::Books;
        if !sq.exact
            && (n < FEW_RESULTS || (wants_names && names(&res) == 0))
            && let Some(fixed) = self.correct(&sq.q)?
            && let Some(fp) = plan(&fixed)
        {
            let alt = self.search_plan(&conn, &fp, sq, src)?;
            // the corrected query is shown instead when the query as typed found little, or no
            // author/series where the corrected one finds some; else it is only offered
            if (n < FEW_RESULTS && found(&alt) > n)
                || (wants_names && names(&res) == 0 && names(&alt) > 0)
            {
                res = alt;
                res.corrected = Some(fixed);
                used = fp.tokens;
            } else if found(&alt) > n {
                res.did_you_mean = Some(fixed);
            }
        }
        res.highlight = highlight(&used, &res);
        res.took_ms = started.elapsed().as_millis() as u64;
        Ok(res)
    }

    /// `q` with the words the catalog does not know (no vocabulary word starts with them, nor
    /// with their transliteration) replaced by the closest known word; `None` when nothing
    /// changes.
    pub fn correct(&self, q: &str) -> Result<Option<String>> {
        let vocab = self.vocab()?;
        if vocab.is_empty() {
            return Ok(None);
        }
        let mut changed = false;
        let words: Vec<String> = search_tokens(q)
            .into_iter()
            .map(|t| {
                if t.chars().count() < 4 || !t.chars().any(char::is_alphabetic) || vocab.knows(&t) {
                    return t;
                }
                match vocab.closest(&t) {
                    Some((w, _)) => {
                        changed = true;
                        w
                    }
                    None => t,
                }
            })
            .collect();
        Ok(changed.then(|| words.join(" ")))
    }

    fn fts_ids(
        conn: &rusqlite::Connection,
        table: &str,
        q: &str,
    ) -> Result<std::collections::HashSet<i64>> {
        let mut st =
            conn.prepare_cached(&format!("SELECT rowid FROM {table} WHERE {table} MATCH ?1"))?;
        Ok(st
            .query_map([q], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn search_plan(
        &self,
        conn: &rusqlite::Connection,
        p: &Plan,
        sq: &SearchQuery,
        src: &dyn crate::rank::RatingSource,
    ) -> Result<SearchResult> {
        let mut res = SearchResult::default();
        let any = p.broad.as_deref().unwrap_or(&p.strict);
        if matches!(sq.kind, SearchKind::All | SearchKind::Authors) {
            let mut st = conn.prepare_cached(
                "SELECT a.id, a.name, a.book_count FROM author a \
                 WHERE a.id IN (SELECT rowid FROM author_fts WHERE author_fts MATCH ?1) \
                 ORDER BY substr(a.sort_key, 1, length(?3)) = ?3 DESC, \
                 a.id IN (SELECT rowid FROM author_fts WHERE author_fts MATCH ?2) DESC, \
                 a.book_count DESC, a.sort_key LIMIT 20",
            )?;
            res.authors = st
                .query_map([any, &p.strict, &p.phrase_key], |r| {
                    Ok(NameCount {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        count: r.get(2)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
        }
        if matches!(sq.kind, SearchKind::All | SearchKind::Series) {
            let mut st = conn.prepare_cached(
                "SELECT s.id, s.name, s.book_count, s.authors FROM series s \
                 WHERE s.id IN (SELECT rowid FROM series_fts WHERE series_fts MATCH ?1) \
                 ORDER BY substr(s.sort_key, 1, length(?3)) = ?3 DESC, \
                 s.id IN (SELECT rowid FROM series_fts WHERE series_fts MATCH ?2) DESC, \
                 s.book_count DESC, s.sort_key LIMIT 20",
            )?;
            res.series = st
                .query_map([any, &p.strict, &p.phrase_key], |r| {
                    Ok(SeriesHit {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        count: r.get(2)?,
                        authors: r.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
        }
        if matches!(sq.kind, SearchKind::All | SearchKind::Books) {
            let attrs = self.attrs()?;
            let mut hits: Vec<(i64, f64)> = {
                // Column weights: title, authors, series, keywords, stems, Latin keys.
                let mut st = conn.prepare_cached(
                    "SELECT rowid, bm25(book_fts, 10.0, 4.0, 3.0, 1.0, 3.0, 2.0) FROM book_fts WHERE book_fts MATCH ?1",
                )?;
                st.query_map([any], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?
            };
            let strict = match &p.broad {
                Some(_) => Some(Self::fts_ids(conn, "book_fts", &p.strict)?),
                None => None,
            };
            let phrase = Self::fts_ids(conn, "book_fts", &p.phrase)?;
            for (id, score) in hits.iter_mut() {
                let tier = if phrase.contains(id) {
                    3.0
                } else if strict.as_ref().is_none_or(|s| s.contains(id)) {
                    2.0
                } else {
                    1.0
                };
                *score -= tier * TIER;
            }
            let hits: Vec<(i64, f64)> = if sq.rating.has_filter() {
                hits.into_iter()
                    .filter(|(id, _)| {
                        (*id as usize) < attrs.flags.len() && sq.rating.accepts(*id, &attrs, src)
                    })
                    .collect()
            } else {
                hits
            };
            let (ranked, total, facets) = filter_and_facet(&attrs, &hits, sq);
            let limit = sq.limit.clamp(1, 1000);
            // with grouping every hit is ordered (groups are cut after grouping)
            let keep = if sq.group { usize::MAX } else { limit };
            let ordered: Vec<i64> = if sq.rating.sort != crate::rank::RatingSort::None {
                let mut keyed: Vec<(u64, f64, i64)> = ranked
                    .into_iter()
                    .map(|(id, score)| (sq.rating.key(id, &attrs, src), score, id))
                    .collect();
                keyed.sort_by(|a, b| {
                    b.0.cmp(&a.0)
                        .then(a.1.total_cmp(&b.1))
                        .then_with(|| attrs.date[b.2 as usize].cmp(&attrs.date[a.2 as usize]))
                        .then(a.2.cmp(&b.2))
                });
                keyed.truncate(keep);
                keyed.into_iter().map(|x| x.2).collect()
            } else {
                top_n(ranked, keep, &attrs)
            };
            if sq.group {
                let mut groups = crate::works::group_ids(&ordered, &attrs, src);
                crate::works::sort_groups(&mut groups, &sq.rating, &attrs, src);
                res.total = groups.len() as i64;
                groups.truncate(limit);
                res.books = crate::works::load_groups(conn, &groups)?;
            } else {
                res.books = load_books(conn, &ordered)?;
                res.total = total;
            }
            res.facets = facets;
        }
        Ok(res)
    }
}

/// Apply filters; count disjunctive facets (each facet ignores its own filter).
fn filter_and_facet(
    a: &BookAttrs,
    hits: &[(i64, f64)],
    sq: &SearchQuery,
) -> (Vec<(i64, f64)>, i64, Facets) {
    let gset: Vec<u16> = {
        let mut v: Vec<u16> = sq
            .genres
            .iter()
            .flat_map(|&g| genres().with_descendants(g))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let lang_ok: Vec<bool> = a
        .langs
        .iter()
        .map(|l| sq.langs.is_empty() || sq.langs.iter().any(|x| x == l))
        .collect();
    let ext_ok: Vec<bool> = a
        .exts
        .iter()
        .map(|e| sq.ext.as_ref().is_none_or(|x| x.eq_ignore_ascii_case(e)))
        .collect();
    let from = sq.from.as_deref().map(date_num).unwrap_or(0);
    let to = sq
        .to
        .as_deref()
        .map(date_num)
        .filter(|&d| d > 0)
        .unwrap_or(u32::MAX);

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
    let mut lang: Vec<(String, i64)> = l_counts
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(i, c)| (a.langs[i].clone(), *c))
        .collect();
    lang.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    let mut ext: Vec<(String, i64)> = e_counts
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(i, c)| (a.exts[i].clone(), *c))
        .collect();
    ext.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    (out, total, Facets { genre, lang, ext })
}

/// Best `n` hits: bm25 ascending (more relevant first), then newer first, then id.
fn top_n(mut v: Vec<(i64, f64)>, n: usize, a: &BookAttrs) -> Vec<i64> {
    let n = n.min(v.len());
    let cmp = |x: &(i64, f64), y: &(i64, f64)| {
        x.1.total_cmp(&y.1)
            .then_with(|| a.date[y.0 as usize].cmp(&a.date[x.0 as usize]))
            .then(x.0.cmp(&y.0))
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
