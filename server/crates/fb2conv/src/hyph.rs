//! Liang (TeX) hyphenation using the pattern dictionaries of the Qt app
//! (`assets/hyphenations/{ru,uk,en,de}.txt`, one or more whitespace-separated patterns per line,
//! e.g. `.аб1р`, `4а3а`). Port of `fb2mobi/hyphenations.cpp` with the usual
//! left/right minimums instead of Qt's ad-hoc syllable checks.

use std::collections::HashMap;

pub const SOFT_HYPHEN: char = '\u{ad}';

pub struct Hyphenator {
    patterns: HashMap<Box<str>, Box<[u8]>>,
    max_len: usize,
    left_min: usize,
    right_min: usize,
}

impl Hyphenator {
    pub fn from_patterns(src: &str) -> Hyphenator {
        let mut patterns = HashMap::with_capacity(8192);
        let mut max_len = 0;
        for pat in src.split_whitespace() {
            let mut letters = String::with_capacity(pat.len());
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
            let n = letters.chars().count();
            if n == 0 || levels.len() != n + 1 {
                continue;
            }
            max_len = max_len.max(n);
            patterns.insert(letters.into_boxed_str(), levels.into_boxed_slice());
        }
        Hyphenator { patterns, max_len, left_min: 2, right_min: 2 }
    }

    /// Returns the char indices (into `word`) before which a hyphen may be inserted.
    pub fn points(&self, word: &str) -> Vec<usize> {
        let n = word.chars().count();
        if n < self.left_min + self.right_min || n > 60 {
            return Vec::new();
        }
        let mut dotted = String::with_capacity(word.len() + 2);
        dotted.push('.');
        for c in word.chars() {
            dotted.extend(c.to_lowercase());
        }
        dotted.push('.');
        // byte offsets of chars in `dotted`, plus the end
        let offs: Vec<usize> = dotted.char_indices().map(|(i, _)| i).chain(std::iter::once(dotted.len())).collect();
        let m = offs.len() - 1; // chars in dotted
        if m != n + 2 {
            // lower-casing changed the length (rare); give up on this word
            return Vec::new();
        }
        let mut levels = vec![0u8; m + 1];
        for i in 0..m {
            let maxj = (i + self.max_len).min(m);
            for j in i + 1..=maxj {
                if let Some(lv) = self.patterns.get(&dotted[offs[i]..offs[j]]) {
                    for (k, &l) in lv.iter().enumerate() {
                        if l > levels[i + k] {
                            levels[i + k] = l;
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
        self.hyphenate_cached(text, out, &mut HashMap::new());
    }

    /// Same as [`Self::hyphenate_text`] with a per-document word cache (words repeat a lot).
    pub fn hyphenate_cached(&self, text: &str, out: &mut String, cache: &mut HashMap<String, Box<str>>) {
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

    fn push_word(&self, w: &str, out: &mut String, cache: &mut HashMap<String, Box<str>>) {
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
