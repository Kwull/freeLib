//! Finding the next book: cover hints for picking the best copy of a work, the start page
//! ("Continue series", "New from authors I read") and followed authors / series.
//!
//! Follows and dismissed series are stored by the author's / series' normalized name
//! (`sort_key`), which survives re-imports (ids do not have to).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use freelib_catalog::home::{NewBook, NewFromSeeds, SeriesNext};
use freelib_catalog::{Book, Catalog, RatingSource};
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::api::browse::{BookOut, with_marks};
use crate::db;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::now_rfc3339;

/// Known cover presence per library and `book_key`, recorded whenever a book's preview is
/// read (first view or cache). Not persisted: after a restart covers become known again as
/// books are viewed.
#[derive(Default)]
pub struct CoverHints {
    known: Mutex<HashMap<i64, HashMap<String, bool>>>,
}

/// At most this many known covers are resolved per request.
const COVER_HINTS_MAX: usize = 20_000;

impl CoverHints {
    pub fn record(&self, lib: i64, key: &str, has: bool) {
        let mut m = self.known.lock().unwrap_or_else(|e| e.into_inner());
        let per = m.entry(lib).or_default();
        if per.len() < 200_000 || per.contains_key(key) {
            per.insert(key.to_string(), has);
        }
    }

    /// Known covers of `lib` as book id → has cover (blocking: one catalog query).
    pub fn ids(&self, lib: i64, cat: &Catalog) -> HashMap<i64, bool> {
        let known: Vec<(String, bool)> = {
            let m = self.known.lock().unwrap_or_else(|e| e.into_inner());
            match m.get(&lib) {
                Some(per) => per
                    .iter()
                    .take(COVER_HINTS_MAX)
                    .map(|(k, v)| (k.clone(), *v))
                    .collect(),
                None => return HashMap::new(),
            }
        };
        let by_key: HashMap<String, bool> = known.into_iter().collect();
        let keys: Vec<String> = by_key.keys().cloned().collect();
        cat.ids_by_keys(&keys)
            .map(|v| {
                v.into_iter()
                    .filter_map(|(k, id)| by_key.get(&k).map(|h| (id, *h)))
                    .collect()
            })
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------- app.db

/// Latest history time per book of `lib`: `book_key → at`.
pub fn history_latest(c: &Connection, uid: i64, lib: i64) -> ApiResult<HashMap<String, String>> {
    let mut st = c.prepare_cached(
        "SELECT book_key, max(at) FROM book_history WHERE user_id=?1 AND library_id=?2 GROUP BY book_key",
    )?;
    let rows = st.query_map(params![uid, lib], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Follows of `lib`: `(kind, key, name)`.
pub fn follows(c: &Connection, uid: i64, lib: i64) -> ApiResult<Vec<(String, String, String)>> {
    let mut st = c.prepare_cached(
        "SELECT kind, key, name FROM follow WHERE user_id=?1 AND library_id=?2 ORDER BY created_at",
    )?;
    let rows = st.query_map(params![uid, lib], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn set_follow(
    c: &Connection,
    uid: i64,
    lib: i64,
    kind: &str,
    key: &str,
    name: &str,
    follow: bool,
) -> ApiResult<()> {
    if follow {
        c.execute(
            "INSERT OR IGNORE INTO follow(user_id, library_id, kind, key, name, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![uid, lib, kind, key, name, now_rfc3339()],
        )?;
    } else {
        c.execute(
            "DELETE FROM follow WHERE user_id=?1 AND library_id=?2 AND kind=?3 AND key=?4",
            params![uid, lib, kind, key],
        )?;
    }
    Ok(())
}

/// Sort keys of the series dismissed from "Continue series".
pub fn dismissed(c: &Connection, uid: i64, lib: i64) -> ApiResult<HashSet<String>> {
    let mut st =
        c.prepare_cached("SELECT key FROM series_dismiss WHERE user_id=?1 AND library_id=?2")?;
    let rows = st.query_map(params![uid, lib], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn set_dismissed(
    c: &Connection,
    uid: i64,
    lib: i64,
    key: &str,
    name: &str,
    dismissed: bool,
) -> ApiResult<()> {
    if dismissed {
        c.execute(
            "INSERT OR REPLACE INTO series_dismiss(user_id, library_id, key, name, at) VALUES (?1,?2,?3,?4,?5)",
            params![uid, lib, key, name, now_rfc3339()],
        )?;
    } else {
        c.execute(
            "DELETE FROM series_dismiss WHERE user_id=?1 AND library_id=?2 AND key=?3",
            params![uid, lib, key],
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------- follows

/// A followed author or series resolved in the current catalog.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Followed {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowList {
    pub authors: Vec<Followed>,
    pub series: Vec<Followed>,
}

/// The user's follows of `lib` resolved to ids (names the catalog no longer has are left out).
pub fn follow_list(st: &AppState, uid: i64, lib: i64, cat: &Catalog) -> ApiResult<FollowList> {
    let rows = {
        let c = st.db.lock();
        follows(&c, uid, lib)?
    };
    let keys = |kind: &str| -> Vec<String> {
        rows.iter()
            .filter(|r| r.0 == kind)
            .map(|r| r.1.clone())
            .collect()
    };
    let mut out = FollowList::default();
    for (_, id) in cat.authors_by_keys(&keys("author"))? {
        if let Some(a) = cat.author(id)? {
            out.authors.push(Followed {
                id,
                name: a.name,
                count: a.count,
            });
        }
    }
    for (_, id) in cat.series_by_keys(&keys("series"))? {
        if let Some(s) = cat.series(id)? {
            out.series.push(Followed {
                id,
                name: s.name,
                count: s.count,
            });
        }
    }
    out.authors.sort_by(|a, b| a.name.cmp(&b.name));
    out.series.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Follows or unfollows an author / series by id.
pub fn follow(
    st: &AppState,
    uid: i64,
    lib: i64,
    cat: &Catalog,
    kind: &str,
    id: i64,
    on: bool,
) -> ApiResult<FollowList> {
    let (key, name) = match kind {
        "author" => (
            cat.author_key(id)?,
            cat.author(id)?.map(|a| a.name).unwrap_or_default(),
        ),
        "series" => (
            cat.series_key(id)?,
            cat.series(id)?.map(|s| s.name).unwrap_or_default(),
        ),
        _ => return Err(ApiError::bad_request("kind must be author or series")),
    };
    let key = key.ok_or_else(|| ApiError::not_found(format!("{kind} not found")))?;
    {
        let c = st.db.lock();
        set_follow(&c, uid, lib, kind, &key, &name, on)?;
    }
    follow_list(st, uid, lib, cat)
}

// ---------------------------------------------------------------- start page

/// Next works shown per started series.
pub const NEXT_PER_SERIES: usize = 2;
/// Started series shown at most.
pub const CONTINUE_MAX: usize = 24;
/// New books shown at most.
pub const NEW_MAX: usize = 60;
/// Suggestions for new users.
pub const PICKS_MAX: usize = 12;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesNextOut {
    #[serde(flatten)]
    pub s: SeriesNextHead,
    pub next: Vec<BookOut>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesNextHead {
    pub series: freelib_catalog::SeriesHit,
    pub works: usize,
    pub done: usize,
    pub last_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewBookOut {
    #[serde(flatten)]
    pub book: BookOut,
    pub reason: freelib_catalog::home::NewReason,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewFromOut {
    /// Books dated on or after this day count as new.
    pub since: String,
    /// The window in days; `None` = since the previous visit.
    pub days: Option<i64>,
    pub total: usize,
    pub books: Vec<NewBookOut>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Home {
    /// The user has no history, ratings, shelves or follows in this library yet.
    pub empty: bool,
    pub continue_series: Vec<SeriesNextOut>,
    pub new_from_authors: NewFromOut,
    /// Suggestions (best-rated recent books) for the empty state.
    pub picks: Vec<BookOut>,
    pub following: FollowCounts,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowCounts {
    pub authors: usize,
    pub series: usize,
}

fn days_ago(days: i64) -> String {
    let now = crate::util::unix_now() / 86_400;
    let (y, m, d) = freelib_catalog::util::civil_from_days(now - days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Builds the start page (blocking). `days`: the "new" window; `None` = since the previous
/// visit (30 days when there was none).
pub fn home(
    st: &AppState,
    uid: i64,
    lib: i64,
    cat: &Catalog,
    days: Option<i64>,
) -> ApiResult<Home> {
    let (history, ratings, shelved, follow_rows, dismissed_keys, prev) = {
        let c = st.db.lock();
        (
            history_latest(&c, uid, lib)?,
            db::user_ratings(&c, uid, lib)?,
            db::user_shelf_books(&c, uid, lib)?,
            follows(&c, uid, lib)?,
            dismissed(&c, uid, lib)?,
            db::prev_visit(&c, uid)?,
        )
    };
    // resolve every book key once
    let mut keys: Vec<String> = history.keys().cloned().collect();
    keys.extend(ratings.iter().map(|r| r.0.clone()));
    keys.extend(shelved.iter().map(|r| r.2.clone()));
    keys.sort();
    keys.dedup();
    let ids: HashMap<String, i64> = cat.ids_by_keys(&keys)?.into_iter().collect();

    // done = sent / downloaded / read, or rated (a rating means it was read)
    let mut done: HashMap<i64, String> = HashMap::new();
    for (k, at) in &history {
        if let Some(id) = ids.get(k) {
            done.insert(*id, at.clone());
        }
    }
    for (k, _) in &ratings {
        if let Some(id) = ids.get(k) {
            done.entry(*id).or_default();
        }
    }
    let src = CoversOnly(st.covers.ids(lib, cat));
    let cont = cat.continue_series(&done, &dismissed_keys, &src, NEXT_PER_SERIES, CONTINUE_MAX)?;

    // "New from authors I read": authors of books done (not just rated), rated ≥ 4, shelved
    let mut seed_books: Vec<i64> = history.keys().filter_map(|k| ids.get(k)).copied().collect();
    seed_books.extend(
        ratings
            .iter()
            .filter(|r| r.1 >= 4)
            .filter_map(|r| ids.get(&r.0)),
    );
    seed_books.extend(shelved.iter().filter_map(|r| ids.get(&r.2)));
    seed_books.sort_unstable();
    seed_books.dedup();
    let fkeys = |kind: &str| -> Vec<String> {
        follow_rows
            .iter()
            .filter(|r| r.0 == kind)
            .map(|r| r.1.clone())
            .collect()
    };
    let seeds = NewFromSeeds {
        followed_authors: cat
            .authors_by_keys(&fkeys("author"))?
            .into_iter()
            .map(|x| x.1)
            .collect(),
        followed_series: cat
            .series_by_keys(&fkeys("series"))?
            .into_iter()
            .map(|x| x.1)
            .collect(),
        read_authors: cat.reading_authors(&seed_books)?,
    };
    let since = match days {
        Some(d) => days_ago(d.clamp(1, 3650)),
        None => prev
            .as_deref()
            .map(|p| p.chars().take(10).collect::<String>())
            .filter(|p| p.len() == 10)
            .unwrap_or_else(|| days_ago(30)),
    };
    let mut exclude: HashSet<i64> = done.keys().copied().collect();
    exclude.extend(shelved.iter().filter_map(|r| ids.get(&r.2)));
    let (new_books, new_total) = cat.new_from(&seeds, &since, &exclude, &src, NEW_MAX)?;

    let empty =
        history.is_empty() && ratings.is_empty() && shelved.is_empty() && follow_rows.is_empty();
    let picks = if empty || (cont.is_empty() && new_books.is_empty()) {
        cat.picks(30, &src, PICKS_MAX)?
    } else {
        Vec::new()
    };

    // user marks for every book shown, in one pass
    let mut all: Vec<Book> = Vec::new();
    for s in &cont {
        all.extend(s.next.iter().cloned());
    }
    all.extend(new_books.iter().map(|n| n.book.clone()));
    all.extend(picks.iter().cloned());
    let mut marked: HashMap<i64, BookOut> = with_marks(st, uid, lib, all)?
        .into_iter()
        .map(|b| (b.book.id, b))
        .collect();
    let mut take = |b: &Book| {
        marked.remove(&b.id).or_else(|| {
            with_marks(st, uid, lib, vec![b.clone()])
                .ok()
                .and_then(|mut v| v.pop())
        })
    };
    let continue_series = cont
        .into_iter()
        .map(|s: SeriesNext| SeriesNextOut {
            next: s.next.iter().filter_map(&mut take).collect(),
            s: SeriesNextHead {
                series: s.series,
                works: s.works,
                done: s.done,
                last_at: s.last_at,
            },
        })
        .collect();
    let books = new_books
        .into_iter()
        .filter_map(|n: NewBook| {
            take(&n.book).map(|book| NewBookOut {
                book,
                reason: n.reason,
            })
        })
        .collect();
    let picks = picks.iter().filter_map(&mut take).collect();
    Ok(Home {
        empty,
        continue_series,
        new_from_authors: NewFromOut {
            since,
            days,
            total: new_total,
            books,
        },
        picks,
        following: FollowCounts {
            authors: seeds.followed_authors.len(),
            series: seeds.followed_series.len(),
        },
    })
}

/// Dismisses (or restores) a series in "Continue series".
pub fn dismiss(
    st: &AppState,
    uid: i64,
    lib: i64,
    cat: &Catalog,
    series: i64,
    on: bool,
) -> ApiResult<()> {
    let key = cat
        .series_key(series)?
        .ok_or_else(|| ApiError::not_found("series not found"))?;
    let name = cat.series(series)?.map(|s| s.name).unwrap_or_default();
    let c = st.db.lock();
    set_dismissed(&c, uid, lib, &key, &name, on)
}

/// Editions of the work of book `id`, best copy first, with marks and notes.
pub fn editions(
    st: &AppState,
    uid: i64,
    lib: i64,
    cat: &Catalog,
    id: i64,
) -> ApiResult<serde_json::Value> {
    let src = CoversOnly(st.covers.ids(lib, cat));
    let eds = cat
        .editions(id, false, &src)?
        .ok_or_else(|| ApiError::not_found("book not found"))?;
    let notes: Vec<Option<String>> = eds.iter().map(|e| e.note.clone()).collect();
    let books = with_marks(st, uid, lib, eds.into_iter().map(|e| e.book).collect())?;
    let rows: Vec<serde_json::Value> = books
        .into_iter()
        .zip(notes)
        .map(|(b, note)| {
            let mut v = serde_json::to_value(&b).unwrap_or_default();
            v["note"] = serde_json::json!(note);
            v
        })
        .collect();
    let best = rows.first().and_then(|b| b["id"].as_i64()).unwrap_or(id);
    Ok(serde_json::json!({ "best": best, "books": rows }))
}

/// A [`RatingSource`] without ratings that knows covers (best copy of a work).
pub struct CoversOnly(pub HashMap<i64, bool>);

impl RatingSource for CoversOnly {
    fn my(&self, _: i64) -> u8 {
        0
    }
    fn ext(&self, _: i64) -> Option<(u16, u32)> {
        None
    }
    fn has_cover(&self, id: i64) -> Option<bool> {
        self.0.get(&id).copied()
    }
}
