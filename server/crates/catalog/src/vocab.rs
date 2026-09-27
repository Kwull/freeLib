//! Typo tolerance: the vocabulary of author, series and title words (`vocab` table, written by
//! the importer with the number of live books per word), held in memory per catalog.
//!
//! A query word that is not the prefix of any vocabulary word (nor, by its Latin key, of any
//! word's key) is looked up by edit distance (≤ 1 for 4–5 letters, ≤ 2 from 6, counted on the
//! shorter of the two words), in the word's own script and in the Latin-key space (so `azimv`
//! finds `азимов`); see [`Vocab::closest`] for what counts as close — nonsense such as
//! `zzzqxw` is not corrected at all. The closest, then most frequent, word wins. Candidates are
//! pre-filtered by length and a 64-bit character set.

use rusqlite::Connection;

use crate::text::{char_sig, edit_distance, has_cyrillic, latin_key, max_typos, translit};

/// Byte ranges into the arenas plus the pre-filter data of one word.
struct Entry {
    w_off: u32,
    w_len: u16,
    k_off: u32,
    k_len: u16,
    w_chars: u8,
    k_chars: u8,
    freq: u32,
    w_sig: u64,
    k_sig: u64,
}

/// The in-memory vocabulary (sorted by word).
pub struct Vocab {
    words: String,
    keys: String,
    entries: Vec<Entry>,
    /// Entry indexes sorted by key.
    by_key: Vec<u32>,
}

impl Vocab {
    pub(crate) fn load(conn: &Connection) -> rusqlite::Result<Vocab> {
        let mut v = Vocab {
            words: String::new(),
            keys: String::new(),
            entries: Vec::new(),
            by_key: Vec::new(),
        };
        let mut st = conn.prepare("SELECT word, freq FROM vocab ORDER BY word")?;
        let mut q = st.query([])?;
        while let Some(r) = q.next()? {
            let w = r.get_ref(0)?.as_str().unwrap_or("");
            let wc = w.chars().count();
            if w.len() > u16::MAX as usize || wc > 255 {
                continue;
            }
            let k = latin_key(w).unwrap_or_default();
            let e = Entry {
                w_off: v.words.len() as u32,
                w_len: w.len() as u16,
                k_off: v.keys.len() as u32,
                k_len: k.len().min(u16::MAX as usize) as u16,
                w_chars: wc as u8,
                k_chars: k.chars().count().min(255) as u8,
                freq: r.get::<_, i64>(1)?.clamp(0, u32::MAX as i64) as u32,
                w_sig: char_sig(w),
                k_sig: char_sig(&k),
            };
            v.words.push_str(w);
            v.keys.push_str(&k[..e.k_len as usize]);
            v.entries.push(e);
        }
        let mut by_key: Vec<u32> = (0..v.entries.len() as u32).collect();
        by_key.sort_by(|a, b| v.key(*a as usize).cmp(v.key(*b as usize)));
        v.by_key = by_key;
        Ok(v)
    }

    fn word(&self, i: usize) -> &str {
        let e = &self.entries[i];
        &self.words[e.w_off as usize..e.w_off as usize + e.w_len as usize]
    }

    fn key(&self, i: usize) -> &str {
        let e = &self.entries[i];
        &self.keys[e.k_off as usize..e.k_off as usize + e.k_len as usize]
    }

    /// Number of words.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the vocabulary is empty (catalogs imported without one).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Approximate heap size in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.words.len()
            + self.keys.len()
            + self.entries.len() * std::mem::size_of::<Entry>()
            + self.by_key.len() * 4
    }

    /// Whether some word starts with `prefix` (normalized).
    pub fn has_prefix(&self, prefix: &str) -> bool {
        let idx = self.lower_bound_word(prefix);
        idx < self.entries.len() && self.word(idx).starts_with(prefix)
    }

    fn lower_bound_word(&self, w: &str) -> usize {
        let (mut lo, mut hi) = (0usize, self.entries.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.word(mid) < w {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Whether some word's Latin key starts with `key`.
    pub fn has_key_prefix(&self, key: &str) -> bool {
        let (mut lo, mut hi) = (0usize, self.by_key.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.key(self.by_key[mid] as usize) < key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo < self.by_key.len() && self.key(self.by_key[lo] as usize).starts_with(key)
    }

    /// Whether a query word is known: some word starts with it, or (3+ letters) some word's
    /// Latin key starts with its key.
    pub fn knows(&self, token: &str) -> bool {
        self.has_prefix(token)
            || latin_key(token).is_some_and(|k| k.len() >= 3 && self.has_key_prefix(&k))
    }

    /// The closest vocabulary word to `token` that is genuinely close to it, by distance, then
    /// frequency; `None` when there is none (or the token is shorter than 4 characters).
    ///
    /// A word is close when, within [`max_typos`] of the *shorter* of the two words (so a long
    /// query never "corrects" into a 3-letter word):
    /// * it is written in the same script and within that many edits (from 2 edits the first
    ///   letter must agree: `zzzqxw` is no typo of anything);
    /// * or it has the same phonetic key (`asimow` → `азимов`);
    /// * or it is in the other script, its phonetic key is within that many edits and so is its
    ///   transliteration (`azimv` → `азимов`) — phonetic keys fold doubled letters and
    ///   spelling variants, so a key match alone is not enough (`zzzqxw` has the key `skxv`, one
    ///   edit from `скв`'s `skv`).
    pub fn closest(&self, token: &str) -> Option<(String, usize)> {
        let t_chars = token.chars().count();
        let max_w = max_typos(t_chars);
        if max_w == 0 {
            return None;
        }
        let key = latin_key(token);
        let k_chars = key.as_deref().map(|k| k.chars().count()).unwrap_or(0);
        let max_k = max_typos(k_chars).min(max_w);
        let t_cyr = has_cyrillic(token);
        let t_first = token.chars().next();
        let k_first = key.as_deref().and_then(|k| k.chars().next());
        let t_lat = translit(token).to_lowercase();
        let (t_sig, k_sig) = (char_sig(token), key.as_deref().map(char_sig).unwrap_or(0));
        // (distance, in the word's own script, freq, entry): closest first; among equals a word
        // of the query's own script (`азимв` → `азимов`, not the Latin `asimov` of the same
        // phonetic key), then the most frequent
        let mut best: Option<(usize, bool, u32, usize)> = None;
        for (i, e) in self.entries.iter().enumerate() {
            let cap = best.map(|b| b.0).unwrap_or(usize::MAX);
            // typos tolerated between these two words: by the shorter one
            let bound = max_typos(t_chars.min(e.w_chars as usize));
            if bound == 0 {
                continue;
            }
            let mut d: Option<usize> = None;
            let mut own = false;
            if (e.w_chars as usize).abs_diff(t_chars) <= bound
                && ((e.w_sig ^ t_sig).count_ones() as usize) <= 2 * bound
            {
                let w = self.word(i);
                d = edit_distance(token, w, bound.min(cap)).filter(|&d| {
                    has_cyrillic(w) == t_cyr && (d < 2 || w.chars().next() == t_first)
                });
                own = d.is_some();
            }
            if let Some(k) = key.as_deref()
                && max_k > 0
                && e.k_len > 0
                && (e.k_chars as usize).abs_diff(k_chars) <= max_k
                && ((e.k_sig ^ k_sig).count_ones() as usize) <= 2 * max_k
            {
                let kb = max_k.min(bound).min(max_typos(e.k_chars as usize));
                let wk = self.key(i);
                if let Some(dk) = edit_distance(k, wk, kb.min(cap))
                    && (dk < 2 || wk.chars().next() == k_first)
                    && (dk == 0
                        || edit_distance(&t_lat, &translit(self.word(i)).to_lowercase(), bound)
                            .is_some())
                {
                    if d.is_none_or(|x| dk < x) {
                        own = false;
                    }
                    d = Some(d.map_or(dk, |x| x.min(dk)));
                }
            }
            let Some(d) = d else { continue };
            if d == 0 && self.word(i) == token {
                continue;
            }
            let better = match best {
                None => true,
                Some((bd, bown, bf, _)) => {
                    d < bd || (d == bd && own && !bown) || (d == bd && own == bown && e.freq > bf)
                }
            };
            if better {
                best = Some((d, own, e.freq, i));
            }
        }
        best.map(|(d, _, _, i)| (self.word(i).to_string(), d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(words: &[(&str, i64)]) -> Vocab {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE vocab (word TEXT PRIMARY KEY, freq INTEGER NOT NULL)")
            .unwrap();
        for (w, f) in words {
            c.execute("INSERT INTO vocab(word, freq) VALUES (?1, ?2)", (w, f))
                .unwrap();
        }
        Vocab::load(&c).unwrap()
    }

    fn fix(v: &Vocab, t: &str) -> Option<String> {
        v.closest(t).map(|x| x.0)
    }

    #[test]
    fn corrects_only_close_words() {
        let v = vocab(&[
            ("азимов", 300),
            ("asimov", 20),
            ("скв", 900),
            ("скат", 50),
            ("пикник", 40),
            ("стругацкий", 200),
            ("лукьяненко", 150),
            ("robert", 60),
            ("квартал", 70),
            ("мир", 5000),
            ("мира", 800),
            ("кот", 400),
        ]);
        // the good cases: a letter dropped, swapped or added; a transliteration with a typo
        assert_eq!(fix(&v, "азимв").as_deref(), Some("азимов"));
        assert_eq!(fix(&v, "азмов").as_deref(), Some("азимов"));
        assert_eq!(fix(&v, "пикнк").as_deref(), Some("пикник"));
        assert_eq!(fix(&v, "стругацкй").as_deref(), Some("стругацкий"));
        assert_eq!(fix(&v, "стругакций").as_deref(), Some("стругацкий"));
        assert_eq!(fix(&v, "лукяненко").as_deref(), Some("лукьяненко"));
        assert_eq!(fix(&v, "robrt").as_deref(), Some("robert"));
        assert_eq!(fix(&v, "azimv").as_deref(), Some("азимов"));
        assert!(fix(&v, "asimow").is_some_and(|w| w == "asimov" || w == "азимов"));
        let v2 = vocab(&[("шекли", 90), ("роберт", 300), ("толстой", 200)]);
        // across scripts: a typo in a transliteration
        assert_eq!(fix(&v2, "robrt").as_deref(), Some("роберт"));
        assert_eq!(fix(&v2, "tolstoi").as_deref(), Some("толстой"));
        // nonsense is never "corrected": the phonetic key of `zzzqxw` (`skxv`) is one edit from
        // `скв`'s, but `скв` is 3 letters and nothing like it
        for t in [
            "zzzqxw",
            "qwrtpl",
            "xqzvbn",
            "ъъъъъ",
            "йцукенгш",
            "ggggggg",
            "zxcvbnm",
            "фывапр",
            "sqkv",
            "zzzz",
            "qxwz",
        ] {
            assert_eq!(fix(&v, t), None, "{t}");
        }
        // short words: nothing below 4 letters, 1 edit for 4–5 letters
        assert_eq!(fix(&v, "мор"), None);
        assert_eq!(fix(&v, "кто"), None);
        assert_eq!(fix(&v, "миро").as_deref(), Some("мира"));
        assert_eq!(fix(&v, "мирно"), None, "2 edits from «мира» (4 letters)");
        // 6+ letters: 2 edits, but the first letter must agree then
        assert_eq!(fix(&v, "бзимов").as_deref(), Some("азимов"), "one edit");
        assert_eq!(
            fix(&v, "бзимоа"),
            None,
            "two edits and another first letter"
        );
        assert_eq!(fix(&v, "азимоааа"), None, "too many edits");
    }
}
