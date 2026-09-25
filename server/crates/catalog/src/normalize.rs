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

/// Sort key used for authors, series and titles.
///
/// Unicode lower-case, `ё→е`, quotes removed, punctuation turned into spaces,
/// whitespace collapsed, leading non-alphanumerics (e.g. `- `, `*`, `#`) removed.
/// `й` is kept distinct from `и`. Comparing two keys with plain byte order
/// (SQLite `BINARY`, Rust `str::cmp`) gives the list order: digits, Latin, Cyrillic.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
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
            out.push(match lc {
                'ё' => 'е',
                other => other,
            });
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
