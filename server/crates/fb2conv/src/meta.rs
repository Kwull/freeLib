//! Book metadata for the EPUB package (and for Calibre): language tags, clean titles, sort
//! keys, the stable identifier, and catalog overrides.

use serde::{Deserialize, Serialize};

/// Metadata the caller knows better than the FB2 file: the catalog's view of the book.
///
/// Every field is optional; `BookMeta::default()` converts the FB2 exactly as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BookMeta {
    /// Stable id of the book in its library (the catalog `book_key`). The EPUB's unique
    /// identifier becomes a UUID derived from it ([`book_uuid`]), so re-sending the same book
    /// replaces the earlier copy on Kindle / Apple Books instead of adding a duplicate.
    pub book_key: Option<String>,
    /// Fallback language (the catalog's), used when the FB2 `<lang>` is missing or invalid.
    pub lang: Option<String>,
    /// Series name from the catalog; wins over the FB2 `<sequence>`.
    pub series: Option<String>,
    /// Number in [`series`](Self::series).
    pub serno: Option<u32>,
}

/// `urn:uuid:…` derived from a catalog book key (name-based, deterministic).
pub fn book_uuid(book_key: &str) -> String {
    format!(
        "urn:uuid:{}",
        crate::epub::uuid_from(crate::epub::hash128(
            format!("freelib-book:{}", book_key.trim()).as_bytes()
        ))
    )
}

/// ISO 639-2 (bibliographic and terminological) and a few spelled-out names → ISO 639-1.
const LANG_ALIASES: &[(&str, &str)] = &[
    ("rus", "ru"),
    ("russian", "ru"),
    ("русский", "ru"),
    ("eng", "en"),
    ("english", "en"),
    ("ukr", "uk"),
    ("ua", "uk"),
    ("ukrainian", "uk"),
    ("українська", "uk"),
    ("bel", "be"),
    ("by", "be"),
    ("ger", "de"),
    ("deu", "de"),
    ("german", "de"),
    ("fra", "fr"),
    ("fre", "fr"),
    ("french", "fr"),
    ("spa", "es"),
    ("ita", "it"),
    ("pol", "pl"),
    ("cze", "cs"),
    ("ces", "cs"),
    ("bul", "bg"),
    ("srp", "sr"),
    ("hrv", "hr"),
    ("slo", "sk"),
    ("slk", "sk"),
    ("lit", "lt"),
    ("lav", "lv"),
    ("est", "et"),
    ("fin", "fi"),
    ("swe", "sv"),
    ("nor", "no"),
    ("dan", "da"),
    ("dut", "nl"),
    ("nld", "nl"),
    ("por", "pt"),
    ("heb", "he"),
    ("iw", "he"),
    ("jpn", "ja"),
    ("chi", "zh"),
    ("zho", "zh"),
    ("kaz", "kk"),
    ("geo", "ka"),
    ("kat", "ka"),
    ("arm", "hy"),
    ("hye", "hy"),
    ("tur", "tr"),
    ("gre", "el"),
    ("ell", "el"),
    ("lat", "la"),
    ("esp", "eo"),
    ("epo", "eo"),
];

/// Normalises a language tag to BCP 47 (`ru`, `en-GB`, `sr-Latn`, `zh-Hant-TW`): `_` → `-`,
/// three-letter and spelled-out codes → two letters, canonical case (language lower, script
/// title, region upper). `None` when the tag is empty or not a plausible language tag.
pub fn normalize_language(tag: &str) -> Option<String> {
    let t = tag.trim().replace('_', "-").to_lowercase();
    if t.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = t.split('-').map(str::to_string).collect();
    if let Some((_, to)) = LANG_ALIASES.iter().find(|(from, _)| *from == parts[0]) {
        parts[0] = (*to).to_string();
    }
    let lang_ok =
        (2..=3).contains(&parts[0].len()) && parts[0].chars().all(|c| c.is_ascii_lowercase());
    let rest_ok = parts[1..]
        .iter()
        .all(|p| !p.is_empty() && p.len() <= 8 && p.chars().all(|c| c.is_ascii_alphanumeric()));
    if !lang_ok || !rest_ok {
        return None;
    }
    let mut out = parts[0].clone();
    for p in &parts[1..] {
        out.push('-');
        if p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic()) {
            // script: Latn, Cyrl
            let mut c = p.chars();
            out.extend(c.next().map(|f| f.to_ascii_uppercase()));
            out.push_str(c.as_str());
        } else if p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic())
            || p.len() == 3 && p.chars().all(|c| c.is_ascii_digit())
        {
            // region: RU, 419
            out.push_str(&p.to_ascii_uppercase());
        } else {
            out.push_str(p);
        }
    }
    Some(out)
}

/// The book language: the FB2 `<lang>`, else `fallback` (the catalog's), else a guess from
/// `sample` (Cyrillic → `ru`, otherwise `en`).
pub fn book_language(fb2_lang: &str, fallback: Option<&str>, sample: &str) -> String {
    normalize_language(fb2_lang)
        .or_else(|| fallback.and_then(normalize_language))
        .unwrap_or_else(|| {
            if sample.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c)) {
                "ru".into()
            } else {
                "en".into()
            }
        })
}

/// Junk that library tools append to titles: file formats and source tags.
const JUNK_TAGS: &[&str] = &[
    "fb2",
    "fb2.zip",
    "epub",
    "rtf",
    "txt",
    "doc",
    "docx",
    "pdf",
    "djvu",
    "html",
    "htm",
    "mobi",
    "azw3",
    "ocr",
    "litres",
    "litres.ru",
];

/// Words that introduce a number in a title ("Книга 3", "Том 2", "Book 3", "#3").
const NUMBER_WORDS: &[&str] = &[
    "книга",
    "кн.",
    "кн",
    "том",
    "т.",
    "часть",
    "ч.",
    "выпуск",
    "вып.",
    "книжка",
    "частина",
    "book",
    "vol.",
    "vol",
    "volume",
    "part",
    "pt.",
    "no.",
    "nr.",
    "band",
    "teil",
    "#",
    "№",
];

/// Cleans a title for metadata: collapses whitespace, drops format tags such as `(fb2)` or
/// `[litres]`, and a trailing series number that repeats `serno` ("Дозор. Книга 3",
/// "Dune (Book 1)", "Title #2"). A bare trailing number is kept ("Метро 2033").
pub fn clean_title(title: &str, serno: Option<u32>) -> String {
    let mut t = title.split_whitespace().collect::<Vec<_>>().join(" ");
    // format / source tags in brackets, anywhere
    loop {
        let before = t.clone();
        for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
            let mut out = String::with_capacity(t.len());
            let mut rest = t.as_str();
            while let Some(i) = rest.find(open) {
                let Some(j) = rest[i..].find(close) else {
                    break;
                };
                let inner = rest[i + 1..i + j].trim().to_lowercase();
                out.push_str(&rest[..i]);
                if !JUNK_TAGS.contains(&inner.as_str()) {
                    out.push_str(&rest[i..=i + j]);
                }
                rest = &rest[i + j + 1..];
            }
            out.push_str(rest);
            t = out.split_whitespace().collect::<Vec<_>>().join(" ");
        }
        if t == before {
            break;
        }
    }
    if let Some(n) = serno {
        t = strip_trailing_number(&t, n);
    }
    let t = t.trim_matches(|c: char| {
        c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-' | '–' | '—')
    });
    // a single trailing period ("Мастер и Маргарита."), not an ellipsis
    let t = if t.ends_with('.') && !t.ends_with("..") && t.chars().count() > 2 {
        &t[..t.len() - 1]
    } else {
        t
    };
    let t = t.trim().to_string();
    if t.is_empty() {
        title.trim().to_string()
    } else {
        t
    }
}

fn strip_trailing_number(t: &str, n: u32) -> String {
    let lower = t.to_lowercase();
    let num = n.to_string();
    // "(Книга 3)" / "[#3]"
    for (open, close) in [('(', ')'), ('[', ']')] {
        if lower.ends_with(close)
            && let Some(i) = lower.rfind(open)
        {
            let inner = lower[i + 1..lower.len() - 1].trim().to_string();
            if number_phrase(&inner, &num) {
                return t[..i].trim_end().to_string();
            }
        }
    }
    // ". Книга 3", " - Том 3", " #3", ", часть 3"
    if let Some(stem) = lower.strip_suffix(num.as_str()) {
        let stem_t = stem.trim_end();
        for w in NUMBER_WORDS {
            if let Some(before) = stem_t.strip_suffix(w)
                && (before.is_empty()
                    || before.ends_with(|c: char| {
                        c.is_whitespace() || matches!(c, '.' | ',' | '-' | '–' | '—' | ':' | ';')
                    }))
                && !before.trim().is_empty()
            {
                let cut = before.len();
                return t[..cut].trim_end().to_string();
            }
        }
    }
    t.to_string()
}

fn number_phrase(inner: &str, num: &str) -> bool {
    let Some(stem) = inner.strip_suffix(num) else {
        return false;
    };
    let stem = stem.trim_end();
    stem.is_empty() || NUMBER_WORDS.contains(&stem)
}

/// Sort form of a title: leading punctuation and quotes removed, an English leading article
/// moved to the end ("The Hobbit" → "Hobbit, The").
pub fn title_sort(title: &str, lang: &str) -> String {
    let t = title
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim_end_matches(['»', '”', '"', '\'', '“'])
        .trim()
        .to_string();
    if lang.split('-').next() == Some("en") {
        for art in ["The ", "A ", "An "] {
            if t.len() > art.len() && t[..art.len()].eq_ignore_ascii_case(art) {
                return format!("{}, {}", t[art.len()..].trim(), art.trim());
            }
        }
    }
    if t.is_empty() {
        title.trim().to_string()
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages() {
        assert_eq!(normalize_language("rus").as_deref(), Some("ru"));
        assert_eq!(normalize_language(" RU ").as_deref(), Some("ru"));
        assert_eq!(normalize_language("ru_ru").as_deref(), Some("ru-RU"));
        assert_eq!(normalize_language("sr-latn").as_deref(), Some("sr-Latn"));
        assert_eq!(normalize_language("es-419").as_deref(), Some("es-419"));
        assert_eq!(normalize_language("Русский").as_deref(), Some("ru"));
        assert_eq!(normalize_language("ukr").as_deref(), Some("uk"));
        assert_eq!(normalize_language(""), None);
        assert_eq!(normalize_language("??"), None);
        assert_eq!(normalize_language("1234"), None);
        assert_eq!(book_language("", Some("uk"), "Привіт"), "uk");
        assert_eq!(book_language("xx-", Some("de"), ""), "de");
        assert_eq!(book_language("", None, "Привет"), "ru");
        assert_eq!(book_language("", None, "Hello"), "en");
        assert_eq!(book_language("en", Some("ru"), ""), "en");
    }

    #[test]
    fn titles() {
        assert_eq!(
            clean_title("  Трудно  быть\tбогом (fb2) ", None),
            "Трудно быть богом"
        );
        assert_eq!(
            clean_title("Пикник на обочине [litres]", None),
            "Пикник на обочине"
        );
        assert_eq!(
            clean_title("Ночной дозор. Книга 1", Some(1)),
            "Ночной дозор"
        );
        assert_eq!(clean_title("Dune (Book 1)", Some(1)), "Dune");
        assert_eq!(clean_title("Title #2", Some(2)), "Title");
        assert_eq!(clean_title("Title #2", Some(3)), "Title #2");
        assert_eq!(clean_title("Метро 2033", Some(2033)), "Метро 2033");
        assert_eq!(
            clean_title("Мастер и Маргарита.", None),
            "Мастер и Маргарита"
        );
        assert_eq!(clean_title("Ну, погоди...", None), "Ну, погоди...");
        assert_eq!(clean_title("Книга 3", Some(3)), "Книга 3");
        assert_eq!(clean_title("(fb2)", None), "(fb2)");
        assert_eq!(clean_title("Капитал (Том 2)", Some(2)), "Капитал");
        assert_eq!(
            clean_title("Война и мир (в 4 томах)", Some(4)),
            "Война и мир (в 4 томах)"
        );
    }

    #[test]
    fn sort_and_uuid() {
        assert_eq!(title_sort("The Hobbit", "en"), "Hobbit, The");
        assert_eq!(title_sort("«Тихий Дон»", "ru"), "Тихий Дон");
        assert_eq!(title_sort("A", "en"), "A");
        let a = book_uuid("lib:12345");
        assert_eq!(a, book_uuid(" lib:12345 "));
        assert_ne!(a, book_uuid("lib:12346"));
        assert!(a.starts_with("urn:uuid:") && a.len() == 9 + 36);
    }
}
