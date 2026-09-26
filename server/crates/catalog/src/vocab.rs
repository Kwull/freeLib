//! Typo tolerance: the vocabulary of author, series and title words (`vocab` table, written by
//! the importer with the number of live books per word), held in memory per catalog.
//!
//! A query word that is not the prefix of any vocabulary word (nor, by its Latin key, of any
//! word's key) is looked up by edit distance (≤ 1 for 4–7 letters, ≤ 2 from 8), in the word's
//! own script and in the Latin-key space (so `azimv` finds `азимов`); the closest, then most
//! frequent, word wins. Candidates are pre-filtered by length and a 64-bit character set.

use rusqlite::Connection;

use crate::text::{char_sig, edit_distance, latin_key, max_typos};

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

    /// The closest vocabulary word to `token` within the tolerated typos, by distance, then
    /// frequency; `None` when there is none (or the token is shorter than 4 characters).
    pub fn closest(&self, token: &str) -> Option<(String, usize)> {
        let t_chars = token.chars().count();
        let max_w = max_typos(t_chars);
        let key = latin_key(token);
        let k_chars = key.as_deref().map(|k| k.chars().count()).unwrap_or(0);
        let max_k = max_typos(k_chars);
        if max_w == 0 && max_k == 0 {
            return None;
        }
        let (t_sig, k_sig) = (char_sig(token), key.as_deref().map(char_sig).unwrap_or(0));
        let mut best: Option<(usize, u32, usize)> = None; // (distance, freq, entry)
        for (i, e) in self.entries.iter().enumerate() {
            let cap = best.map(|b| b.0).unwrap_or(usize::MAX);
            let mut d: Option<usize> = None;
            if max_w > 0
                && (e.w_chars as usize).abs_diff(t_chars) <= max_w
                && ((e.w_sig ^ t_sig).count_ones() as usize) <= 2 * max_w
            {
                d = edit_distance(token, self.word(i), max_w.min(cap));
            }
            if let Some(k) = key.as_deref()
                && max_k > 0
                && e.k_len > 0
                && (e.k_chars as usize).abs_diff(k_chars) <= max_k
                && ((e.k_sig ^ k_sig).count_ones() as usize) <= 2 * max_k
                && let Some(dk) = edit_distance(k, self.key(i), max_k.min(cap))
            {
                d = Some(d.map_or(dk, |x| x.min(dk)));
            }
            let Some(d) = d else { continue };
            if d == 0 && self.word(i) == token {
                continue;
            }
            let better = match best {
                None => true,
                Some((bd, bf, _)) => d < bd || (d == bd && e.freq > bf),
            };
            if better {
                best = Some((d, e.freq, i));
            }
        }
        best.map(|(d, _, i)| (self.word(i).to_string(), d))
    }
}
