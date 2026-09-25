//! Sort-key normalisation shared by the importer, the read API and search.

/// Characters dropped entirely (quotes, apostrophes): `O'Brien` → `obrien`.
fn is_dropped(c: char) -> bool {
    matches!(
        c,
        '\'' | '"' | '`' | '«' | '»' | '„' | '“' | '”' | '‘' | '’' | '‚' | '‹' | '›' | '´'
    )
}

/// Punctuation treated as a word separator: `Толстой Л.Н.` → `толстой л н`.
fn is_separator(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '.' | ','
                | ':'
                | ';'
                | '!'
                | '?'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '…'
                | '—'
                | '–'
                | '/'
                | '\\'
                | '|'
        )
}

/// Base letters of accented / special lower-case Latin letters (`č → c`, `ø → o`, `ß → ss`,
/// `ə → e`), so that the letter index and sorting treat them like their base letter.
/// Cyrillic is never folded (`й` and `ё` are handled separately). Keep in sync with
/// `FOLD_GROUPS` in web/src/lib/utils/normalize.ts.
fn fold_latin(c: char) -> Option<&'static str> {
    if c < '\u{c0}' {
        return None;
    }
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'ǎ' | 'ǟ' | 'ǡ' | 'ǻ' | 'ȁ' | 'ȃ'
        | 'ȧ' | 'ḁ' | 'ạ' | 'ả' | 'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ' | 'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ' => {
            "a"
        }
        'æ' => "ae",
        'ƀ' | 'ḃ' | 'ḅ' | 'ḇ' => "b",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' | 'ḉ' => "c",
        'ð' | 'ď' | 'đ' | 'ḋ' | 'ḍ' | 'ḏ' | 'ḑ' | 'ḓ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'ǝ' | 'ȅ' | 'ȇ' | 'ȩ' | 'ə' | 'ḕ'
        | 'ḗ' | 'ḙ' | 'ḛ' | 'ḝ' | 'ẹ' | 'ẻ' | 'ẽ' | 'ế' | 'ề' | 'ể' | 'ễ' | 'ệ' => {
            "e"
        }
        'ƒ' | 'ḟ' => "f",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' | 'ǥ' | 'ǧ' | 'ǵ' | 'ḡ' => "g",
        'ĥ' | 'ħ' | 'ȟ' | 'ḣ' | 'ḥ' | 'ḧ' | 'ḩ' | 'ḫ' | 'ẖ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' | 'ǐ' | 'ȉ' | 'ȋ' | 'ɨ' | 'ḭ' | 'ḯ'
        | 'ỉ' | 'ị' => "i",
        'ĵ' | 'ǰ' => "j",
        'ķ' | 'ĸ' | 'ǩ' | 'ḱ' | 'ḳ' | 'ḵ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' | 'ḷ' | 'ḹ' | 'ḻ' | 'ḽ' => "l",
        'ḿ' | 'ṁ' | 'ṃ' => "m",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŋ' | 'ǹ' | 'ṅ' | 'ṇ' | 'ṉ' | 'ṋ' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'ơ' | 'ǒ' | 'ǫ' | 'ǭ' | 'ȍ' | 'ȏ'
        | 'ȫ' | 'ȭ' | 'ȯ' | 'ȱ' | 'ṍ' | 'ṏ' | 'ṑ' | 'ṓ' | 'ọ' | 'ỏ' | 'ố' | 'ồ' | 'ổ' | 'ỗ'
        | 'ộ' | 'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ' => "o",
        'œ' => "oe",
        'ṕ' | 'ṗ' => "p",
        'ŕ' | 'ŗ' | 'ř' | 'ȑ' | 'ȓ' | 'ṙ' | 'ṛ' | 'ṝ' | 'ṟ' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' | 'ṡ' | 'ṣ' | 'ṥ' | 'ṧ' | 'ṩ' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' | 'ț' | 'ṫ' | 'ṭ' | 'ṯ' | 'ṱ' | 'ẗ' => "t",
        'þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'ư' | 'ǔ' | 'ǖ' | 'ǘ' | 'ǚ'
        | 'ǜ' | 'ȕ' | 'ȗ' | 'ṳ' | 'ṵ' | 'ṷ' | 'ṹ' | 'ṻ' | 'ụ' | 'ủ' | 'ứ' | 'ừ' | 'ử' | 'ữ'
        | 'ự' => "u",
        'ṽ' | 'ṿ' => "v",
        'ŵ' | 'ẁ' | 'ẃ' | 'ẅ' | 'ẇ' | 'ẉ' | 'ẘ' => "w",
        'ẋ' | 'ẍ' => "x",
        'ý' | 'ÿ' | 'ŷ' | 'ȳ' | 'ẏ' | 'ẙ' | 'ỳ' | 'ỵ' | 'ỷ' | 'ỹ' => "y",
        'ź' | 'ż' | 'ž' | 'ƶ' | 'ȥ' | 'ẑ' | 'ẓ' | 'ẕ' => "z",
        _ => return None,
    })
}

/// Sort key used for authors, series and titles.
///
/// Unicode lower-case, `ё→е`, accented Latin letters folded to their base letters
/// (`Čapek` → `capek`, `Ødegaard` → `odegaard`, combining accents after a Latin letter dropped),
/// quotes removed, punctuation turned into spaces, whitespace collapsed, leading
/// non-alphanumerics (e.g. `- `, `*`, `#`) removed. `й` is kept distinct from `и`. Comparing two keys with plain byte order
/// (SQLite `BINARY`, Rust `str::cmp`) gives the list order: digits, Latin, Cyrillic.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        // A combining accent after a (folded) Latin letter: `e\u{301}` → `e`.
        if ('\u{300}'..='\u{36f}').contains(&c) && out.ends_with(|p: char| p.is_ascii_alphabetic())
        {
            continue;
        }
        if is_dropped(c) {
            continue;
        }
        if is_separator(c) {
            pending_space = true;
            continue;
        }
        if out.is_empty() && !c.is_alphanumeric() {
            // Skip leading dashes, asterisks, etc.
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        for lc in c.to_lowercase() {
            match lc {
                'ё' => out.push('е'),
                other => match fold_latin(other) {
                    Some(f) => out.push_str(f),
                    None => out.push(other),
                },
            }
        }
    }
    if out.is_empty() {
        // Only punctuation: keep something stable so that the row still sorts deterministically.
        let t = s.trim().to_lowercase();
        return t.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    out
}

/// Index letter of a sort key: first character upper-cased when it is a letter, `#` otherwise.
pub fn letter_of(sort_key: &str) -> String {
    match sort_key.chars().next() {
        Some(c) if c.is_alphabetic() => {
            let up: String = c.to_uppercase().collect();
            if up == "Ё" { "Е".to_string() } else { up }
        }
        _ => "#".to_string(),
    }
}

/// Split free text into search tokens the same way FTS5 `unicode61` does
/// (runs of alphanumeric characters), normalised with [`normalize`].
pub fn search_tokens(q: &str) -> Vec<String> {
    q.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(normalize)
        .filter(|w| !w.is_empty())
        .collect()
}

/// Collapse internal whitespace and trim (used for display names).
pub fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lower_and_yo() {
        assert_eq!(normalize("Ёлкин Пётр"), "елкин петр");
        assert_eq!(normalize("ЁЖИК"), "ежик");
        assert_eq!(normalize("Йозеф"), "йозеф");
    }

    #[test]
    fn punctuation_and_quotes() {
        assert_eq!(normalize("«Война и мир»"), "война и мир");
        assert_eq!(normalize("Толстой Л.Н."), "толстой л н");
        assert_eq!(normalize("O'Brien"), "obrien");
        assert_eq!(normalize("Что? Где? Когда!"), "что где когда");
        assert_eq!(normalize("Мамин-Сибиряк"), "мамин-сибиряк");
        assert_eq!(normalize("(Не)везучий [сборник]"), "не везучий сборник");
    }

    #[test]
    fn whitespace() {
        assert_eq!(
            normalize("  Стругацкий\t Аркадий \u{a0} Натанович  "),
            "стругацкий аркадий натанович"
        );
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   "), "");
    }

    #[test]
    fn leading_garbage() {
        assert_eq!(normalize("- Hello"), "hello");
        assert_eq!(normalize("...и всё"), "и все");
        assert_eq!(normalize("#1"), "1");
        assert_eq!(normalize("!!!"), "!!!");
    }

    #[test]
    fn order_is_bytewise() {
        let mut v = vec![
            normalize("Яков"),
            normalize("Абрамов"),
            normalize("Ёжиков"),
            normalize("Zeta"),
            normalize("1984"),
            normalize("Ежов"),
        ];
        v.sort();
        assert_eq!(v, vec!["1984", "zeta", "абрамов", "ежиков", "ежов", "яков"]);
    }

    #[test]
    fn latin_diacritics_fold() {
        assert_eq!(normalize("Čapek Karel"), "capek karel");
        assert_eq!(normalize("Lem Stanisław"), "lem stanislaw");
        assert_eq!(normalize("Ødegaard Knut"), "odegaard knut");
        assert_eq!(normalize("Əlibəyli"), "elibeyli");
        assert_eq!(normalize("Þórðarson"), "thordarson");
        assert_eq!(normalize("Straße"), "strasse");
        assert_eq!(normalize("Ðukić"), "dukic");
        assert_eq!(normalize("Cafe\u{301}"), "cafe");
        // Cyrillic is untouched
        assert_eq!(normalize("Йозеф Їжак Ґанна"), "йозеф їжак ґанна");
        assert_eq!(letter_of(&normalize("Åsa")), "A");
        assert_eq!(letter_of(&normalize("Ə")), "E");
    }

    #[test]
    fn letters() {
        assert_eq!(letter_of("ежик"), "Е");
        assert_eq!(letter_of("ёж"), "Е");
        assert_eq!(letter_of("abc"), "A");
        assert_eq!(letter_of("1984"), "#");
        assert_eq!(letter_of(""), "#");
    }

    #[test]
    fn tokens() {
        assert_eq!(
            search_tokens("Стругацкий, Пикник!"),
            vec!["стругацкий", "пикник"]
        );
        assert_eq!(search_tokens("мамин-сибиряк"), vec!["мамин", "сибиряк"]);
        assert_eq!(search_tokens("  "), Vec::<String>::new());
        assert_eq!(search_tokens("ёж"), vec!["еж"]);
    }
}
