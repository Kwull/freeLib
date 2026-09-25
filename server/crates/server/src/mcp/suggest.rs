//! `suggest_candidates`: server-side scoring of reading suggestions. The model makes the final
//! choice; this ranks a few hundred candidates with plain reasons.
//!
//! Seeds (explicit book ids and/or the user's profile: well-rated, shelved, sent, downloaded or
//! read books) give weights to authors, series and genres. Candidates are the other books of the
//! top seed authors, the following books of seed series, and well-rated books of the top seed
//! genres. Score (all parts additive, each explained in `reasons`):
//!
//! * same author: `3 × author weight / max author weight`
//! * series: `+5` for the next unread number after the last one read, `+2` for later ones
//! * shared genres: `2 × Σ genre weight / max genre weight` over the candidate's genres (≤ 2)
//! * library rating: `(stars − 3) × 0.5` when rated
//! * Open Library: `(average − 3.5) × 1.0` with ≥ 5 votes, `× 0.5` with fewer
//! * negative author weight (the user rated that author's books ≤ 2) subtracts.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

/// Anthologies (this many authors or more) do not make their authors "same author" seeds.
pub const ANTHOLOGY_MIN_AUTHORS: usize = freelib_catalog::ANTHOLOGY_MIN_AUTHORS;

/// What is known about one book for scoring.
#[derive(Debug, Clone, Default)]
pub struct BookFacts {
    pub id: i64,
    pub title: String,
    /// (author id, display name) in INPX order.
    pub authors: Vec<(i64, String)>,
    /// (series id, series name, number)
    pub series: Option<(i64, String, Option<i64>)>,
    pub genres: Vec<u16>,
    /// Library rating 0..5.
    pub stars: u8,
    /// Open Library (average, votes).
    pub ext: Option<(f64, u32)>,
}

/// A seed with its weight (> 0 liked, < 0 disliked).
#[derive(Debug, Clone)]
pub struct Seed {
    pub book: BookFacts,
    pub weight: f64,
    /// Why it is a seed ("you rated it 5", "on shelf Favourites", "you asked").
    pub why: String,
}

/// Weights derived from the seeds.
#[derive(Debug, Default)]
pub struct Profile {
    /// author id → (weight, name, example seed title)
    pub authors: HashMap<i64, (f64, String, String)>,
    /// genre id → weight
    pub genres: HashMap<u16, f64>,
    /// series id → (highest number seen among the seeds, name)
    pub series: HashMap<i64, (i64, String)>,
}

impl Profile {
    pub fn from_seeds(seeds: &[Seed]) -> Profile {
        let mut p = Profile::default();
        for s in seeds {
            let b = &s.book;
            if b.authors.len() < ANTHOLOGY_MIN_AUTHORS {
                for (id, name) in &b.authors {
                    let e = p
                        .authors
                        .entry(*id)
                        .or_insert((0.0, name.clone(), b.title.clone()));
                    e.0 += s.weight;
                }
            }
            if s.weight > 0.0 {
                let share = s.weight / b.genres.len().max(1) as f64;
                for g in &b.genres {
                    *p.genres.entry(*g).or_default() += share;
                }
                if let Some((sid, name, num)) = &b.series {
                    let e = p.series.entry(*sid).or_insert((0, name.clone()));
                    e.0 = e.0.max(num.unwrap_or(0));
                }
            }
        }
        p
    }

    /// The best-weighted authors (liked ones only).
    pub fn top_authors(&self, n: usize) -> Vec<i64> {
        let mut v: Vec<(i64, f64)> = self
            .authors
            .iter()
            .filter(|(_, w)| w.0 > 0.0)
            .map(|(id, w)| (*id, w.0))
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.into_iter().take(n).map(|x| x.0).collect()
    }

    /// The best-weighted genres.
    pub fn top_genres(&self, n: usize) -> Vec<u16> {
        let mut v: Vec<(u16, f64)> = self.genres.iter().map(|(g, w)| (*g, *w)).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.into_iter().take(n).map(|x| x.0).collect()
    }
}

/// A scored candidate.
#[derive(Debug, Clone, Serialize)]
pub struct Scored {
    pub id: i64,
    pub score: f64,
    pub reasons: Vec<String>,
}

/// Scores `candidates` against `profile`; best first, at most `limit`. `genre_name` renders
/// genre ids in reasons.
pub fn score(
    profile: &Profile,
    candidates: &[BookFacts],
    genre_name: &dyn Fn(u16) -> String,
    limit: usize,
) -> Vec<Scored> {
    let max_author = profile
        .authors
        .values()
        .map(|w| w.0)
        .fold(0.0f64, f64::max)
        .max(1e-9);
    let max_genre = profile
        .genres
        .values()
        .copied()
        .fold(0.0f64, f64::max)
        .max(1e-9);
    let mut out: Vec<Scored> = Vec::with_capacity(candidates.len());
    let mut seen = HashSet::new();
    for c in candidates {
        if !seen.insert(c.id) {
            continue;
        }
        let mut s = 0.0;
        let mut reasons = Vec::new();
        // authors (anthologies count only a little)
        let anth = c.authors.len() >= ANTHOLOGY_MIN_AUTHORS;
        let mut best_author: Option<(f64, &String, &String)> = None;
        let mut disliked = 0.0f64;
        for (aid, _) in &c.authors {
            if let Some((w, name, example)) = profile.authors.get(aid) {
                if *w > 0.0 {
                    if best_author.is_none_or(|b| *w > b.0) {
                        best_author = Some((*w, name, example));
                    }
                } else {
                    disliked = disliked.min(*w);
                }
            }
        }
        if let Some((w, name, example)) = best_author {
            let part = 3.0 * w / max_author * if anth { 0.3 } else { 1.0 };
            s += part;
            reasons.push(format!("same author as \"{example}\": {name}"));
        }
        if disliked < 0.0 {
            s += disliked.max(-3.0);
            reasons.push("an author you rated low".into());
        }
        // series continuation
        if let Some((sid, sname, num)) = &c.series
            && let Some((seen_num, _)) = profile.series.get(sid)
        {
            match num {
                Some(n) if *n == seen_num + 1 => {
                    s += 5.0;
                    reasons.push(format!(
                        "next in series \"{sname}\": #{n} after #{seen_num}"
                    ));
                }
                Some(n) if *n > *seen_num => {
                    s += 2.0;
                    reasons.push(format!("later in series \"{sname}\": #{n}"));
                }
                Some(_) => {}
                None => {
                    s += 1.0;
                    reasons.push(format!("same series \"{sname}\""));
                }
            }
        }
        // genres
        let mut gsum = 0.0;
        let mut shared = Vec::new();
        for g in &c.genres {
            if let Some(w) = profile.genres.get(g) {
                gsum += w;
                shared.push(genre_name(*g));
            }
        }
        if gsum > 0.0 {
            s += (2.0 * gsum / max_genre).min(2.0);
            reasons.push(format!("shared genres: {}", shared.join(", ")));
        }
        if c.stars > 0 {
            s += (c.stars as f64 - 3.0) * 0.5;
            if c.stars >= 4 {
                reasons.push(format!("library rating {}/5", c.stars));
            }
        }
        if let Some((avg, votes)) = c.ext
            && votes > 0
        {
            let k = if votes >= 5 { 1.0 } else { 0.5 };
            s += (avg - 3.5) * k;
            if avg >= 4.0 {
                reasons.push(format!("Open Library {avg:.1} ({votes} votes)"));
            }
        }
        if reasons.is_empty() {
            continue;
        }
        out.push(Scored {
            id: c.id,
            score: (s * 100.0).round() / 100.0,
            reasons,
        });
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(id: i64, authors: &[i64], series: Option<(i64, i64)>, genres: &[u16]) -> BookFacts {
        BookFacts {
            id,
            title: format!("Book {id}"),
            authors: authors
                .iter()
                .map(|a| (*a, format!("Author {a}")))
                .collect(),
            series: series.map(|(s, n)| (s, format!("Series {s}"), Some(n))),
            genres: genres.to_vec(),
            stars: 0,
            ext: None,
        }
    }

    fn seed(b: BookFacts, w: f64) -> Seed {
        Seed {
            book: b,
            weight: w,
            why: String::new(),
        }
    }

    #[test]
    fn ranks_series_author_genre() {
        let seeds = vec![
            seed(book(1, &[10], Some((100, 1)), &[101]), 3.0),
            seed(book(2, &[10], Some((100, 2)), &[101]), 2.0),
            seed(book(3, &[20], None, &[400]), -2.0), // disliked author 20
        ];
        let p = Profile::from_seeds(&seeds);
        assert_eq!(p.top_authors(5), vec![10]);
        assert_eq!(p.series[&100].0, 2);
        let mut unrelated_good = book(9, &[99], None, &[700]);
        unrelated_good.stars = 5;
        let mut genre_ol = book(8, &[98], None, &[101]);
        genre_ol.ext = Some((4.6, 300));
        let cands = vec![
            book(4, &[10], Some((100, 3)), &[101]), // next in series, same author
            book(5, &[10], Some((100, 5)), &[101]), // later in series
            book(6, &[10], None, &[200]),           // same author only
            book(7, &[20], None, &[101]),           // disliked author, shared genre
            genre_ol,
            unrelated_good,
            book(4, &[10], Some((100, 3)), &[101]), // duplicate
        ];
        let names = |g: u16| format!("g{g}");
        let r = score(&p, &cands, &names, 10);
        let ids: Vec<i64> = r.iter().map(|s| s.id).collect();
        assert_eq!(ids[0], 4, "{r:?}");
        assert_eq!(ids[1], 5);
        assert!(r[0].reasons.iter().any(|x| x.contains("next in series")));
        assert!(r[0].reasons.iter().any(|x| x.contains("same author")));
        let pos = |id| ids.iter().position(|x| *x == id).unwrap();
        assert!(pos(6) < pos(7), "disliked author ranks lower");
        assert!(
            r.iter()
                .find(|s| s.id == 7)
                .unwrap()
                .reasons
                .iter()
                .any(|x| x.contains("rated low"))
        );
        assert!(
            r.iter()
                .find(|s| s.id == 8)
                .unwrap()
                .reasons
                .iter()
                .any(|x| x.contains("Open Library"))
        );
        assert_eq!(ids.iter().filter(|x| **x == 4).count(), 1, "deduplicated");
        // a well-rated book with nothing in common only gets the rating reason
        let nine = r.iter().find(|s| s.id == 9).unwrap();
        assert_eq!(nine.reasons, vec!["library rating 5/5".to_string()]);
        assert_eq!(score(&p, &cands, &names, 2).len(), 2);
    }

    #[test]
    fn anthologies_do_not_seed_authors() {
        let seeds = vec![seed(book(1, &[1, 2, 3, 4], None, &[101]), 3.0)];
        let p = Profile::from_seeds(&seeds);
        assert!(p.top_authors(5).is_empty());
        assert_eq!(p.top_genres(3), vec![101]);
    }
}
