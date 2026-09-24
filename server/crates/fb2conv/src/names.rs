//! File-name templates (`%a %fa %s %n %b %l %y`), sanitising and Russian → Latin
//! transliteration (port of `Transliteration()` from the Qt `utilites.cpp`).

/// Values available to file-name and cover-label templates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameFields {
    /// First author's last name.
    pub author_last: String,
    pub author_first: String,
    pub author_middle: String,
    pub series: Option<String>,
    pub serno: Option<u32>,
    pub title: String,
    /// Language code, e.g. `ru`.
    pub lang: String,
    /// Date (`YYYY-MM-DD` or anything starting with a 4-digit year).
    pub date: String,
}

impl NameFields {
    /// `Last F.`
    pub fn author_short(&self) -> String {
        let mut s = self.author_last.trim().to_string();
        if let Some(c) = self.author_first.trim().chars().next() {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push(c);
            s.push('.');
        }
        s
    }

    /// `Last First Middle`
    pub fn author_full(&self) -> String {
        [&self.author_last, &self.author_first, &self.author_middle]
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn year(&self) -> String {
        let d = self.date.trim();
        let digits: String = d.chars().take(4).collect();
        if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_digit()) { digits } else { String::new() }
    }
}

const MARK: char = '\u{1}';

fn is_sep(c: char) -> bool {
    matches!(c, ' ' | '-' | '–' | '—' | '_' | '.' | ',' | ':' | ';' | '#' | '№' | '\u{a0}') || c == MARK
}

/// Expands placeholders; empty values leave a marker for [`collapse`].
fn substitute(template: &str, f: &NameFields) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut it = template.chars().peekable();
    let push_val = |out: &mut String, v: String| {
        let v = v.trim().to_string();
        if v.is_empty() { out.push(MARK) } else { out.push_str(&v) }
    };
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.peek().copied() {
            Some('f') => {
                it.next();
                if it.peek() == Some(&'a') {
                    it.next();
                    push_val(&mut out, f.author_full());
                } else {
                    out.push_str("%f");
                }
            }
            Some('a') => {
                it.next();
                push_val(&mut out, f.author_short());
            }
            Some('s') => {
                it.next();
                push_val(&mut out, f.series.clone().unwrap_or_default());
            }
            Some('n') => {
                it.next();
                let width = match it.peek().and_then(|c| c.to_digit(10)) {
                    Some(d) => {
                        it.next();
                        d as usize
                    }
                    None => 2,
                };
                let v = match f.serno {
                    Some(n) if n > 0 => format!("{n:0width$}"),
                    _ => String::new(),
                };
                push_val(&mut out, v);
            }
            Some('b') => {
                it.next();
                push_val(&mut out, f.title.clone());
            }
            Some('l') => {
                it.next();
                push_val(&mut out, f.lang.clone());
            }
            Some('y') => {
                it.next();
                push_val(&mut out, f.year());
            }
            Some('%') => {
                it.next();
                out.push('%');
            }
            _ => out.push('%'),
        }
    }
    out
}

/// Removes empty placeholders together with their separators, within one path component.
fn collapse(component: &str) -> String {
    // 1. empty bracket pairs around missing values: "(\u1)", "[ \u1 ]"
    let mut s = component.to_string();
    if s.contains(MARK) {
        for (o, c) in [('(', ')'), ('[', ']'), ('{', '}'), ('«', '»'), ('"', '"'), ('<', '>')] {
            while let Some(start) = s.find(o) {
                let Some(rel) = s[start + o.len_utf8()..].find(c) else { break };
                let end = start + o.len_utf8() + rel;
                let inner = &s[start + o.len_utf8()..end];
                if inner.chars().all(is_sep) && inner.contains(MARK) {
                    s.replace_range(start..end + c.len_utf8(), &MARK.to_string());
                } else {
                    break;
                }
            }
        }
    }
    // 2. separator runs that contain a missing value collapse to their first separator
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // a dot right after a letter ends an abbreviation ("Last F.") rather than separating
        if !is_sep(chars[i]) || (chars[i] == '.' && i > 0 && chars[i - 1].is_alphanumeric()) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_sep(chars[i]) {
            i += 1;
        }
        let run: String = chars[start..i].iter().collect();
        if !run.contains(MARK) {
            out.push_str(&run);
            continue;
        }
        let clean = run.replace(MARK, " ");
        let first = clean.split_whitespace().next();
        let lead = run.starts_with(|c: char| c.is_whitespace());
        let trail = run.ends_with(|c: char| c.is_whitespace()) || run.ends_with(MARK);
        match first {
            None => out.push(' '),
            Some(tok) => {
                if lead {
                    out.push(' ');
                }
                out.push_str(tok);
                if trail {
                    out.push(' ');
                }
            }
        }
    }
    let t = out.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | '_' | '.' | ',' | ':' | ';' | '#' | '№'));
    let t = t.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | '_' | ',' | ':' | ';' | '#' | '№'));
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Expands a template (e.g. a cover label `"%s %n"`) without file-system sanitising.
pub fn expand(template: &str, f: &NameFields) -> String {
    let s = substitute(template, f);
    s.split('/').map(collapse).filter(|c| !c.is_empty()).collect::<Vec<_>>().join("/")
}

/// `dir`: trailing dots are removed from directory names (Windows drops them silently).
fn sanitize_component(c: &str, dir: bool) -> String {
    let mut out = String::with_capacity(c.len());
    for ch in c.chars() {
        match ch {
            ':' => out.push('.'),
            '"' => out.push('\''),
            '|' => out.push('-'),
            '?' | '*' | '<' | '>' | '\\' => {}
            c if c.is_control() => {}
            '\u{a0}' | '\u{2000}'..='\u{200b}' => out.push(' '),
            c => out.push(c),
        }
    }
    let joined = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = joined.trim_start_matches(['.', ' ']).trim_end_matches(' ');
    let t = if dir { t.trim_end_matches(['.', ' ']) } else { t };
    // keep components within common file-system limits (255 bytes) leaving room for extensions
    let mut end = t.len().min(200);
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    let t = t[..end].trim_end_matches(' ');
    if dir { t.trim_end_matches(['.', ' ']).to_string() } else { t.to_string() }
}

/// Builds a relative file path (without extension) from a template, e.g.
/// `"%a/%s/%n %b"` → `"Стругацкий А./Полдень/03 Трудно быть богом"`.
/// Missing parts collapse with their separators; `/` makes sub-folders; the result is safe
/// for common file systems. Never returns an empty string.
pub fn file_name(template: &str, fields: &NameFields, transliterate: bool) -> String {
    let template = template.replace('\\', "/");
    let s = substitute(&template, fields);
    let comps: Vec<String> = s
        .split('/')
        .map(collapse)
        .map(|c| if transliterate { transliteration(&c) } else { c })
        .filter(|c| !c.is_empty())
        .collect();
    let n = comps.len();
    let parts: Vec<String> = comps
        .iter()
        .enumerate()
        .map(|(i, c)| sanitize_component(c, i + 1 < n))
        .filter(|c| !c.is_empty() && c != "." && c != "..")
        .collect();
    if parts.is_empty() {
        let fallback = if fields.title.trim().is_empty() { "book".to_string() } else { fields.title.clone() };
        let fb = sanitize_component(&if transliterate { transliteration(&fallback) } else { fallback }, false);
        return if fb.is_empty() { "book".into() } else { fb };
    }
    parts.join("/")
}

/// Russian (and Ukrainian/Belarusian) → Latin, as in the Qt app: ASCII passes through,
/// Cyrillic letters are replaced, `?`, `*`, `~` become `.`, everything else is dropped.
pub fn transliteration(s: &str) -> String {
    const UPPER: &str = "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЫЭЮЯ";
    const LAT_UPPER: [&str; 31] = [
        "A", "B", "V", "G", "D", "E", "Jo", "Zh", "Z", "I", "J", "K", "L", "M", "N", "O", "P", "R", "S", "T", "U", "F",
        "H", "C", "Ch", "Sh", "Sh", "I", "E", "Ju", "Ja",
    ];
    let mut out = String::with_capacity(s.len());
    for ch in s.trim().chars() {
        if ch.is_ascii_alphanumeric() || " -_,.()[]{}!@#$%^&+=/'".contains(ch) {
            out.push(ch);
            continue;
        }
        match ch {
            '?' | '*' | '~' => {
                out.push('.');
                continue;
            }
            'Ъ' | 'ъ' | 'Ь' | 'ь' => continue,
            'І' => out.push('I'),
            'і' => out.push('i'),
            'Ї' => out.push_str("Ji"),
            'ї' => out.push_str("ji"),
            'Є' => out.push_str("Je"),
            'є' => out.push_str("je"),
            'Ґ' => out.push('G'),
            'ґ' => out.push('g'),
            'Ў' => out.push('U'),
            'ў' => out.push('u'),
            '«' | '»' | '“' | '”' | '„' => out.push('\''),
            '—' | '–' => out.push('-'),
            '№' => out.push('N'),
            c if c.is_whitespace() => out.push(' '),
            c => {
                let upper = c.to_uppercase().next().unwrap_or(c);
                if let Some(idx) = UPPER.chars().position(|u| u == upper) {
                    let lat = LAT_UPPER[idx];
                    if c == upper {
                        out.push_str(lat);
                    } else {
                        out.push_str(&lat.to_lowercase());
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> NameFields {
        NameFields {
            author_last: "Стругацкий".into(),
            author_first: "Аркадий".into(),
            author_middle: "Натанович".into(),
            series: Some("Мир Полудня".into()),
            serno: Some(3),
            title: "Трудно быть богом".into(),
            lang: "ru".into(),
            date: "2007-03-01".into(),
        }
    }

    #[test]
    fn full() {
        let f = fields();
        assert_eq!(file_name("%a/%s/%n %b", &f, false), "Стругацкий А/Мир Полудня/03 Трудно быть богом");
        assert_eq!(file_name("%fa - %b (%y) [%l]", &f, false), "Стругацкий Аркадий Натанович - Трудно быть богом (2007) [ru]");
        assert_eq!(file_name("%a - %b", &f, true), "Strugackij A. - Trudno bit bogom");
    }

    #[test]
    fn missing() {
        let mut f = fields();
        f.series = None;
        f.serno = None;
        assert_eq!(file_name("%a/%s/%n %b", &f, false), "Стругацкий А/Трудно быть богом");
        assert_eq!(file_name("%a - %s %n - %b", &f, false), "Стругацкий А. - Трудно быть богом");
        assert_eq!(file_name("%a - [%s #%n] %b", &f, false), "Стругацкий А. - Трудно быть богом");
        assert_eq!(file_name("%s, %n. %b", &f, false), "Трудно быть богом");
        assert_eq!(expand("%s %n", &f), "");
        f.title = "Что? Где: Когда*".into();
        assert_eq!(file_name("%b", &f, false), "Что Где. Когда");
        f.title.clear();
        f.author_last.clear();
        f.author_first.clear();
        assert_eq!(file_name("%a/%b", &f, false), "book");
    }

    #[test]
    fn translit() {
        assert_eq!(transliteration("Щука и ёж, Объект"), "Shuka i jozh, Obekt");
        assert_eq!(transliteration("Їжак"), "Jizhak");
    }
}
