//! Language helpers for search and edition grouping: Snowball stemming (Russian, English, plus a
//! light Ukrainian suffix stripper), Russian/Ukrainian → Latin keys that fold the usual
//! transliteration variants (`Strugatsky` = `Strugatskii` = `Стругацкий`), a bounded edit
//! distance for typo tolerance, and the work key / edition note of a title.

use std::sync::LazyLock;

use rust_stemmers::{Algorithm, Stemmer};

use crate::normalize::normalize;

/// Russian / Ukrainian → Latin (a simple, common scheme).
pub fn translit(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        let t: &str = match lower {
            'а' => "a",
            'б' => "b",
            'в' => "v",
            'г' => "g",
            'ґ' => "g",
            'д' => "d",
            'е' => "e",
            'ё' => "e",
            'є' => "ye",
            'ж' => "zh",
            'з' => "z",
            'и' => "i",
            'і' => "i",
            'ї' => "yi",
            'й' => "y",
            'к' => "k",
            'л' => "l",
            'м' => "m",
            'н' => "n",
            'о' => "o",
            'п' => "p",
            'р' => "r",
            'с' => "s",
            'т' => "t",
            'у' => "u",
            'ф' => "f",
            'х' => "kh",
            'ц' => "ts",
            'ч' => "ch",
            'ш' => "sh",
            'щ' => "shch",
            'ъ' | 'ь' => "",
            'ы' => "y",
            'э' => "e",
            'ю' => "yu",
            'я' => "ya",
            _ => {
                out.push(c);
                continue;
            }
        };
        if c.is_uppercase() {
            let mut it = t.chars();
            if let Some(f) = it.next() {
                out.extend(f.to_uppercase());
                out.push_str(it.as_str());
            }
        } else {
            out.push_str(t);
        }
    }
    out
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u')
}

/// Latin comparison key of one word: transliterated, accents folded, lower case, and the usual
/// transliteration variants folded (`iy`/`ii`/`ij`/`yi` → `y`, `ks` → `x`, `kh` → `h`,
/// `ts`/`tz`/`tc` → `c`, `tch` → `ch`, `y`/`j` between vowels dropped, `y`/`j` before a vowel
/// after a consonant → `i`, word-initial `ye` → `e`, final `oi`/`oy`/`oj` → `oi`).
pub fn word_key(w: &str) -> String {
    let t = translit(w);
    let folded = normalize(&t);
    let s: String = folded
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let mut s = s
        .replace("tch", "ch")
        .replace("kh", "h")
        .replace("ks", "x")
        .replace("tz", "c")
        .replace("ts", "c")
        .replace("tc", "c")
        .replace('j', "y")
        .replace("iy", "y")
        .replace("ii", "y")
        .replace("yi", "y");
    if let Some(rest) = s.strip_prefix("ye") {
        s = format!("e{rest}");
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(chars.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == 'y' && i > 0 {
            let prev = chars[i - 1];
            let next = chars.get(i + 1).copied();
            match next {
                Some(n) if is_vowel(n) && is_vowel(prev) => continue, // oye → oe
                Some(n) if is_vowel(n) => {
                    out.push('i'); // kya → kia
                    continue;
                }
                None if is_vowel(prev) => {
                    out.push('i'); // tolstoy → tolstoi
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

// ------------------------------------------------------------------ stemming

static RU: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::create(Algorithm::Russian));
static EN: LazyLock<Stemmer> = LazyLock::new(|| Stemmer::create(Algorithm::English));

fn is_cyrillic(c: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&c)
}

/// Whether `s` contains a Cyrillic letter.
pub fn has_cyrillic(s: &str) -> bool {
    s.chars().any(is_cyrillic)
}

/// Ukrainian endings, longest first (a light stemmer: Snowball has no Ukrainian algorithm).
const UK_ENDINGS: &[&str] = &[
    "ість", "ями", "ами", "ові", "еві", "єві", "ого", "ому", "ими", "іми", "ити", "ати", "ій",
    "их", "іх", "ах", "ях", "ам", "ям", "ом", "ем", "єм", "ою", "ею", "єю", "ти", "ть", "ла", "ло",
    "ли", "ий", "а", "я", "о", "е", "є", "у", "ю", "і", "и", "ї", "й", "ь",
];

fn stem_uk(w: &str) -> String {
    let n = w.chars().count();
    for e in UK_ENDINGS {
        if let Some(s) = w.strip_suffix(e)
            && n - e.chars().count() >= 3
        {
            return s.to_string();
        }
    }
    w.to_string()
}

/// Stem of one normalized word: words with Ukrainian-specific letters (`і ї є ґ`) → a light
/// Ukrainian suffix stripper, other Cyrillic → Snowball Russian, Latin → Snowball English
/// (Porter2). Words shorter than 3 characters and numbers are returned unchanged.
pub fn stem(w: &str) -> String {
    if w.chars().count() < 3 || !w.chars().any(char::is_alphabetic) {
        return w.to_string();
    }
    let s = if w.chars().any(|c| matches!(c, 'і' | 'ї' | 'є' | 'ґ')) {
        stem_uk(w)
    } else if has_cyrillic(w) {
        RU.stem(w).into_owned()
    } else if w.chars().all(|c| c.is_ascii_alphanumeric()) {
        EN.stem(w).into_owned()
    } else {
        return w.to_string();
    };
    // never stem below 2 characters
    if s.chars().count() < 2 {
        w.to_string()
    } else {
        s
    }
}

/// Words of normalized text (runs of alphanumerics, like the FTS tokenizer).
pub fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
}

/// FTS text of the stems of normalized `text` that differ from their word, deduplicated
/// (words equal to their stem are already in the plain columns).
pub fn stems_text(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for w in words(text) {
        let s = stem(w);
        if s != w && !out.contains(&s) {
            out.push(s);
        }
    }
    out.join(" ")
}

/// Latin key of a normalized word for search: [`word_key`] for words of at least 3 characters
/// with a letter, else `None`.
pub fn latin_key(w: &str) -> Option<String> {
    if w.chars().count() < 3 || !w.chars().any(char::is_alphabetic) {
        return None;
    }
    let k = word_key(w);
    (k.len() >= 2).then_some(k)
}

/// FTS text of the Latin keys of normalized `text` that differ from their word.
pub fn latin_text(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for w in words(text) {
        if let Some(k) = latin_key(w)
            && k != w
            && !out.contains(&k)
        {
            out.push(k);
        }
    }
    out.join(" ")
}

// ------------------------------------------------------------------ edit distance

/// Optimal-string-alignment distance (Levenshtein plus adjacent transpositions) of two words,
/// or `None` when it exceeds `max`.
pub fn edit_distance(a: &str, b: &str, max: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > max {
        return None;
    }
    let mut prev2 = vec![0usize; m + 1];
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(prev2[j - 2] + 1);
            }
            cur[j] = v;
            row_min = row_min.min(v);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    (prev[m] <= max).then_some(prev[m])
}

/// Typos tolerated in a word of `len` characters: none below 4, one up to 7, two from 8.
pub fn max_typos(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// 64-bit set of the characters of a word — a cheap pre-filter: one edit changes at most two
/// bits, so words within distance `d` differ in at most `2d` bits.
pub fn char_sig(w: &str) -> u64 {
    let mut s = 0u64;
    for c in w.chars() {
        s |= 1u64 << ((c as u32).wrapping_mul(2_654_435_761) >> 26);
    }
    s
}

// ------------------------------------------------------------------ works and editions

/// Parts of a bracketed note that mark an edition rather than another work: a (different)
/// translation, a revision, abridgement, illustrations, OCR, the self-published mark (`СИ`).
/// Matched against the normalized note followed by a space.
const EDITION_MARKERS: &[&str] = &[
    "перевод",
    "пер ",
    "переводчик",
    "translat",
    "редакц",
    "ред ",
    "версия",
    "version",
    "издани",
    "изд ",
    "edition",
    "испр",
    "дополн",
    "сокращ",
    "abridged",
    "иллюстр",
    "illustr",
    "ocr",
    "полная",
    "полный",
    "другой",
    "другая",
    "альтернатив",
    "вариант",
    "самиздат",
    "журнальн",
    "черновик",
    "litres",
    "с картинками",
    "адаптир",
];

/// Whole normalized notes that mark an edition.
const EDITION_NOTES: &[&str] = &[
    "си", "пер", "ред", "изд", "fb2", "fb3", "epub", "pdf", "djvu", "txt", "rtf", "doc", "docx",
    "html",
];

fn is_edition_note(note: &str) -> bool {
    let n = normalize(note);
    if n.is_empty() {
        return false;
    }
    let padded = format!("{n} ");
    EDITION_NOTES.contains(&n.as_str()) || EDITION_MARKERS.iter().any(|m| padded.contains(m))
}

/// Splits trailing bracketed groups off a title: `("Солярис", ["другой перевод"])` for
/// `Солярис (другой перевод)`. Only groups at the very end are split.
fn trailing_groups(title: &str) -> (String, Vec<String>) {
    let mut t = title.trim().to_string();
    let mut notes = Vec::new();
    loop {
        let open_ch = match t.chars().last() {
            Some(')') => '(',
            Some(']') => '[',
            _ => break,
        };
        let Some(open) = t.rfind(open_ch) else { break };
        let rest = t[..open].trim_end().to_string();
        if rest.is_empty() {
            break;
        }
        notes.push(t[open + 1..t.len() - 1].trim().to_string());
        t = rest;
    }
    notes.reverse();
    (t, notes)
}

/// The edition note of a title: its trailing bracketed groups that mark an edition
/// (`"другой перевод"`, `"пер. Н. Галь"`), joined with `; `, or `None`.
pub fn edition_note(title: &str) -> Option<String> {
    let (_, notes) = trailing_groups(title);
    let v: Vec<String> = notes.into_iter().filter(|n| is_edition_note(n)).collect();
    (!v.is_empty()).then(|| v.join("; "))
}

/// Normalized titles too generic to identify a work: two "Избранное" of one author are usually
/// different selections, so they are never grouped.
const GENERIC_TITLES: &[&str] = &[
    "избранное",
    "сборник",
    "рассказы",
    "повести",
    "повести и рассказы",
    "рассказы и повести",
    "стихотворения",
    "стихи",
    "сочинения",
    "собрание сочинений",
    "полное собрание сочинений",
    "избранные произведения",
    "избранные сочинения",
    "поэмы",
    "пьесы",
    "статьи",
    "эссе",
    "вибране",
    "оповідання",
    "вірші",
    "твори",
    "collected works",
    "selected works",
    "stories",
    "short stories",
    "collected stories",
    "poems",
    "essays",
    "works",
];

/// Work key of a title: normalized, with trailing edition notes (`(другой перевод)`,
/// `[иллюстрации]`, `(СИ)`, `(fb2)`, `[litres]`) removed; other bracketed text (`(сборник)`,
/// `(Часть 2)`) is kept because it can tell works apart. `None` for titles too short or too
/// generic to group. See [`work_title_key_in_series`] for books with a series number.
pub fn work_title_key(title: &str) -> Option<String> {
    work_title_key_in_series(title, None)
}

/// [`work_title_key`] of a book numbered `serno` in its series: a trailing volume marker that
/// only repeats that number (`Основание. Книга 3` as #3) is dropped, any other volume marker
/// (`Том 1` of #37) is kept. Words mixing Cyrillic and Latin look-alike letters (`Oснование`
/// typed with a Latin `O`) are read as Cyrillic.
pub fn work_title_key_in_series(title: &str, serno: Option<i64>) -> Option<String> {
    let (mut t, notes) = trailing_groups(title);
    for n in notes.iter().filter(|n| !is_edition_note(n)) {
        t.push_str(" (");
        t.push_str(n);
        t.push(')');
    }
    let mut k = fold_lookalikes(&normalize(&t));
    if let Some(n) = serno.filter(|&n| n > 0) {
        if let Some((head, v)) = split_trailing_volume(&k) {
            if v == n as u64 && head.chars().filter(|c| c.is_alphanumeric()).count() >= 2 {
                k = head.to_string();
            }
        }
    }
    if k.chars().filter(|c| c.is_alphanumeric()).count() < 2 || GENERIC_TITLES.contains(&k.as_str())
    {
        return None;
    }
    Some(k)
}

/// Words that introduce a volume / part number in a title (normalized).
const VOLUME_WORDS: &[&str] = &[
    "том",
    "т",
    "книга",
    "кн",
    "часть",
    "ч",
    "выпуск",
    "вып",
    "частина",
    "книжка",
    "volume",
    "vol",
    "part",
    "book",
    "tome",
    "band",
    "bd",
];

fn roman(s: &str) -> Option<u64> {
    const R: &[(&str, u64)] = &[
        ("i", 1),
        ("ii", 2),
        ("iii", 3),
        ("iv", 4),
        ("v", 5),
        ("vi", 6),
        ("vii", 7),
        ("viii", 8),
        ("ix", 9),
        ("x", 10),
        ("xi", 11),
        ("xii", 12),
    ];
    R.iter().find(|(r, _)| *r == s).map(|(_, v)| *v)
}

fn volume_value(w: &str) -> Option<u64> {
    if !w.is_empty() && w.len() <= 4 && w.bytes().all(|b| b.is_ascii_digit()) {
        return w.parse().ok();
    }
    roman(w)
}

/// `("люди за спиной", 1)` for the normalized title `люди за спиной том 1`.
fn split_trailing_volume(k: &str) -> Option<(&str, u64)> {
    let (rest, num) = k.rsplit_once(' ')?;
    let v = volume_value(num)?;
    let (head, word) = rest.rsplit_once(' ').unwrap_or(("", rest));
    VOLUME_WORDS.contains(&word).then_some((head.trim_end(), v))
}

/// The volume / part number a title names (`Люди за спиной. Том 1` → 1, `Миры Айзека Азимова.
/// Книга 9` → 9), from the last `<volume word> <number>` pair; `None` when there is none.
/// Two books of one series number naming different volumes are never one work.
pub fn volume_number(title: &str) -> Option<u64> {
    let k = fold_lookalikes(&normalize(title));
    let w: Vec<&str> = k.split(' ').collect();
    (1..w.len()).rev().find_map(|i| {
        VOLUME_WORDS
            .contains(&w[i - 1])
            .then(|| volume_value(w[i]))
            .flatten()
    })
}

/// Latin letters that look like Cyrillic ones, read as Cyrillic inside a word that has
/// Cyrillic letters (`oснование` → `основание`).
fn fold_lookalikes(k: &str) -> String {
    if !k.chars().any(|c| c.is_ascii_alphabetic()) || !has_cyrillic(k) {
        return k.to_string();
    }
    k.split(' ')
        .map(|w| {
            if w.chars().any(|c| c.is_ascii_alphabetic()) && has_cyrillic(w) {
                w.chars()
                    .map(|c| match c {
                        'a' => 'а',
                        'c' => 'с',
                        'e' => 'е',
                        'o' => 'о',
                        'p' => 'р',
                        'x' => 'х',
                        'y' => 'у',
                        'k' => 'к',
                        'm' => 'м',
                        't' => 'т',
                        'h' => 'н',
                        'b' => 'в',
                        other => other,
                    })
                    .collect()
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_word_forms_share_a_stem() {
        let s = stem("книга");
        for w in ["книги", "книгу", "книгой", "книге", "книгам"] {
            assert_eq!(stem(w), s, "{w}");
        }
        assert_eq!(stem("стругацкие"), stem("стругацкий"));
        assert_eq!(stem("толстого"), stem("толстой"));
        assert_eq!(stem("dragons"), stem("dragon"));
        assert_eq!(stem("running"), "run");
        assert_ne!(stem("книжка"), s);
        // Ukrainian (light stemmer)
        assert_eq!(stem("їжака"), stem("їжаку"));
        assert_eq!(stem("пригодами"), stem("пригоди"));
        // short words and numbers are kept
        assert_eq!(stem("её"), "её");
        assert_eq!(stem("1984"), "1984");
    }

    #[test]
    fn stems_and_latin_texts() {
        assert_eq!(stems_text("мир книги"), "книг");
        assert_eq!(stems_text("мир"), "");
        assert_eq!(latin_text("стругацкий аркадий"), "strugacky arkady");
        assert_eq!(latin_text("asimov"), "");
        assert_eq!(latin_key("strugatsky"), Some("strugacky".into()));
        assert_eq!(latin_key("strugatskii"), Some("strugacky".into()));
        assert_eq!(latin_key("стругацкие"), latin_key("strugackie"));
        assert_eq!(latin_key("ив"), None);
    }

    #[test]
    fn distance() {
        assert_eq!(edit_distance("стругацкй", "стругацкий", 2), Some(1));
        assert_eq!(edit_distance("азимв", "азимов", 1), Some(1));
        assert_eq!(edit_distance("abcd", "abdc", 1), Some(1));
        assert_eq!(edit_distance("abcd", "wxyz", 2), None);
        assert_eq!(edit_distance("same", "same", 0), Some(0));
        let (a, b) = (char_sig("стругацкй"), char_sig("стругацкий"));
        assert!((a ^ b).count_ones() <= 2);
        assert_eq!(max_typos(3), 0);
        assert_eq!(max_typos(5), 1);
        assert_eq!(max_typos(9), 2);
    }

    #[test]
    fn work_keys() {
        assert_eq!(work_title_key("Солярис"), Some("солярис".into()));
        assert_eq!(
            work_title_key("Солярис (другой перевод)"),
            Some("солярис".into())
        );
        assert_eq!(
            work_title_key("Солярис [пер. Д. Брускин]"),
            Some("солярис".into())
        );
        assert_eq!(work_title_key("Солярис (СИ)"), Some("солярис".into()));
        assert_eq!(
            work_title_key("Ёлка (иллюстрации автора)"),
            Some("елка".into())
        );
        // other notes tell works apart
        assert_eq!(
            work_title_key("Дозор (Часть 2)"),
            Some("дозор часть 2".into())
        );
        assert_ne!(
            work_title_key("Дозор (Часть 2)"),
            work_title_key("Дозор (Часть 1)")
        );
        assert_eq!(
            work_title_key("Рассказы (сборник)"),
            Some("рассказы сборник".into())
        );
        // generic titles never group
        assert_eq!(work_title_key("Избранное"), None);
        assert_eq!(work_title_key("Стихотворения (другая редакция)"), None);
        assert_eq!(work_title_key("?"), None);
        assert_eq!(
            edition_note("Солярис (другой перевод)"),
            Some("другой перевод".into())
        );
        assert_eq!(edition_note("Дозор (Часть 2)"), None);
        assert_eq!(edition_note("(Не)везучий"), None);
    }

    #[test]
    fn work_keys_normalise_titles() {
        let k = |t: &str| work_title_key(t);
        // punctuation, quotes, case, ё/е, whitespace
        assert_eq!(k("«Люди за спиной». Том 1"), k("Люди за спиной, том  1"));
        assert_eq!(k("ЛЮДИ ЗА СПИНОЙ — ТОМ 1"), k("Люди за спиной. Том 1"));
        assert_eq!(k("Ёжик в тумане"), k("Ежик в тумане"));
        // format and shop notes
        assert_eq!(k("Основание (fb2)"), k("Основание"));
        assert_eq!(k("Основание [litres]"), k("Основание"));
        assert_eq!(k("Основание [Litres] (другой перевод)"), k("Основание"));
        // Latin look-alikes in a Cyrillic word (Latin O, a, e)
        assert_eq!(k("Oснованиe"), k("Основание"));
        assert_eq!(k("Кaмeнская"), k("Каменская"));
        assert_eq!(k("Foundation"), Some("foundation".into()));
        // volumes stay apart
        assert_ne!(k("Люди за спиной. Том 1"), k("Люди за спиной. Том 2"));
        assert_ne!(k("Люди за спиной. Том 1"), k("Люди за спиной"));
        // a trailing volume marker that repeats the series number is dropped
        let s = |t: &str, n: i64| work_title_key_in_series(t, Some(n));
        assert_eq!(s("Основание. Книга 3", 3), s("Основание", 3));
        assert_eq!(s("Основание (Книга 3)", 3), s("Основание", 3));
        assert_ne!(s("Основание. Книга 2", 3), s("Основание", 3));
        assert_eq!(s("Люди за спиной. Том 1", 37), k("Люди за спиной. Том 1"));
        assert_eq!(s("Книга 3", 3), Some("книга 3".into()));
        // omnibus volumes keep their number
        assert_eq!(
            s("Миры Айзека Азимова. Книга 5", 1),
            Some("миры айзека азимова книга 5".into())
        );
    }

    #[test]
    fn volume_numbers() {
        assert_eq!(volume_number("Люди за спиной. Том 1"), Some(1));
        assert_eq!(volume_number("Люди за спиной, т. 2"), Some(2));
        assert_eq!(volume_number("Миры Айзека Азимова. Книга 9"), Some(9));
        assert_eq!(volume_number("Война и мир. Том III"), Some(3));
        assert_eq!(volume_number("The Lord of the Rings, Part 2"), Some(2));
        assert_eq!(volume_number("Академия. Книги 1-7"), None);
        assert_eq!(volume_number("Академия на краю гибели"), None);
        assert_eq!(volume_number("1984"), None);
    }
}
