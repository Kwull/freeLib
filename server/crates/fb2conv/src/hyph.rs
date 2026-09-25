//! Liang (TeX) hyphenation using the pattern dictionaries of the Qt app
//! (`assets/hyphenations/{ru,uk,en,de}.txt`, one or more whitespace-separated patterns per line,
//! e.g. `.аб1р`, `4а3а`). Port of `fb2mobi/hyphenations.cpp` with the usual
//! left/right minimums instead of Qt's ad-hoc syllable checks.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Pass-through hasher for keys that already are 64-bit hashes.
#[derive(Default, Clone, Copy)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x100000001b3);
        }
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = i;
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type IdMap<V> = HashMap<u64, V, BuildHasherDefault<IdHasher>>;

/// Word cache used by [`Hyphenator::hyphenate_cached`].
pub type WordCache = HashMap<String, Box<str>>;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;

#[inline]
fn step(h: u64, c: char) -> u64 {
    // FNV-1a over the code point, then a final avalanche so the map's bucket bits are random
    let h = (h ^ c as u64).wrapping_mul(0x100000001b3);
    h ^ (h >> 29)
}

pub const SOFT_HYPHEN: char = '\u{ad}';

/// Liang hyphenator. Patterns are keyed by an incremental hash of their letters; every prefix of
/// a pattern is also stored (with an empty value) so lookups stop as soon as no pattern can match.
pub struct Hyphenator {
    patterns: IdMap<Box<[u8]>>,
    max_len: usize,
    left_min: usize,
    right_min: usize,
}

impl Hyphenator {
    pub fn from_patterns(src: &str) -> Hyphenator {
        let mut patterns: IdMap<Box<[u8]>> = IdMap::default();
        let mut max_len = 0;
        for pat in src.split_whitespace() {
            let mut letters: Vec<char> = Vec::with_capacity(pat.len());
            let mut levels: Vec<u8> = Vec::with_capacity(pat.len() + 1);
            let mut pending_digit = false;
            for c in pat.chars() {
                if let Some(d) = c.to_digit(10) {
                    levels.push(d as u8);
                    pending_digit = true;
                } else {
                    if !pending_digit {
                        levels.push(0);
                    }
                    pending_digit = false;
                    letters.extend(c.to_lowercase());
                }
            }
            if !pending_digit {
                levels.push(0);
            }
            let n = letters.len();
            if n == 0 || levels.len() != n + 1 {
                continue;
            }
            max_len = max_len.max(n);
            let mut h = FNV_OFFSET;
            for (i, &c) in letters.iter().enumerate() {
                h = step(h, c);
                if i + 1 < n {
                    patterns.entry(h).or_insert_with(|| Box::new([]));
                }
            }
            patterns.insert(h, levels.into_boxed_slice());
        }
        Hyphenator {
            patterns,
            max_len,
            left_min: 2,
            right_min: 2,
        }
    }

    /// Returns the char indices (into `word`) before which a hyphen may be inserted.
    pub fn points(&self, word: &str) -> Vec<usize> {
        let mut dotted: Vec<char> = Vec::with_capacity(word.len() / 2 + 2);
        dotted.push('.');
        for c in word.chars() {
            let mut lc = c.to_lowercase();
            match (lc.next(), lc.next()) {
                (Some(l), None) => dotted.push(l),
                _ => return Vec::new(), // lower-casing changes the length (rare)
            }
        }
        dotted.push('.');
        let m = dotted.len();
        let n = m - 2;
        if n < self.left_min + self.right_min || n > 60 {
            return Vec::new();
        }
        let mut levels = [0u8; 64];
        for i in 0..m {
            let maxj = (i + self.max_len).min(m);
            let mut h = FNV_OFFSET;
            for &c in &dotted[i..maxj] {
                h = step(h, c);
                match self.patterns.get(&h) {
                    None => break,
                    Some(lv) => {
                        for (k, &l) in lv.iter().enumerate() {
                            if l > levels[i + k] {
                                levels[i + k] = l;
                            }
                        }
                    }
                }
            }
        }
        // levels[k] is the value between dotted chars k-1 and k; word char index = k-1
        let mut pts = Vec::new();
        for (k, &l) in levels.iter().enumerate().take(n + 1).skip(1) {
            let idx = k - 1; // hyphen before word char idx
            if l % 2 == 1 && idx >= self.left_min && idx + self.right_min <= n {
                pts.push(idx);
            }
        }
        pts
    }

    /// Hyphenates every word (run of letters) in `text` with soft hyphens.
    pub fn hyphenate_text(&self, text: &str, out: &mut String) {
        self.hyphenate_cached(text, out, &mut WordCache::new());
    }

    /// Same as [`Self::hyphenate_text`] with a per-document word cache (words repeat a lot).
    pub fn hyphenate_cached(&self, text: &str, out: &mut String, cache: &mut WordCache) {
        let mut word_start: Option<usize> = None;
        for (i, c) in text.char_indices() {
            if c.is_alphabetic() {
                if word_start.is_none() {
                    word_start = Some(i);
                }
            } else {
                if let Some(s) = word_start.take() {
                    self.push_word(&text[s..i], out, cache);
                }
                out.push(c);
            }
        }
        if let Some(s) = word_start {
            self.push_word(&text[s..], out, cache);
        }
    }

    fn push_word(&self, w: &str, out: &mut String, cache: &mut WordCache) {
        // fewer than 4 chars: nothing to do
        if w.len() < 4 || (w.len() < 8 && w.chars().count() < self.left_min + self.right_min) {
            out.push_str(w);
            return;
        }
        if let Some(h) = cache.get(w) {
            out.push_str(h);
            return;
        }
        let pts = self.points(w);
        let mut res = String::with_capacity(w.len() + pts.len() * 2);
        let mut pi = 0;
        for (ci, c) in w.chars().enumerate() {
            if pi < pts.len() && pts[pi] == ci {
                res.push(SOFT_HYPHEN);
                pi += 1;
            }
            res.push(c);
        }
        out.push_str(&res);
        if cache.len() < 200_000 {
            cache.insert(w.to_string(), res.into_boxed_str());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_classic() {
        let h = Hyphenator::from_patterns(include_str!("../assets/hyphenations/en.txt"));
        let mut s = String::new();
        h.hyphenate_text("hyphenation", &mut s);
        assert_eq!(s.replace(SOFT_HYPHEN, "-"), "hy-phen-ation");
    }

    #[test]
    fn russian() {
        let h = Hyphenator::from_patterns(include_str!("../assets/hyphenations/ru.txt"));
        let mut s = String::new();
        h.hyphenate_text("Перевоплощение, молоко!", &mut s);
        let v = s.replace(SOFT_HYPHEN, "-");
        assert!(v.contains('-'), "{v}");
        assert!(v.ends_with(", мо-ло-ко!"), "{v}");
    }
}
