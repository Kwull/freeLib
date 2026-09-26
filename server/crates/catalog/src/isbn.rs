//! ISBNs as printed in FB2 `<publish-info><isbn>`: often several in one field
//! (`5-17-012345-0; 978-5-17-012345-2`), with an `ISBN` / `ISBN-13:` prefix, odd dashes or
//! spaces, a lower-case `x`, or a wrong check digit. [`parse`] finds the valid ones.

use serde::Serialize;

/// One valid ISBN in both forms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Isbn {
    /// 13 digits, no separators.
    pub isbn13: String,
    /// 10 characters (last may be `X`), no separators; only for `978-` ISBNs.
    pub isbn10: Option<String>,
    /// As printed, when it was printed with separators (`978-5-699-12014-7`), otherwise the
    /// bare ISBN-13.
    pub display: String,
}

fn check10(d: &[u8]) -> bool {
    d.len() == 10
        && d.iter()
            .enumerate()
            .map(|(i, &v)| (10 - i as u32) * v as u32)
            .sum::<u32>()
            % 11
            == 0
}

fn check13_digit(d12: &[u8]) -> u8 {
    let s: u32 = d12
        .iter()
        .enumerate()
        .map(|(i, &v)| if i % 2 == 0 { v as u32 } else { 3 * v as u32 })
        .sum();
    ((10 - s % 10) % 10) as u8
}

fn check13(d: &[u8]) -> bool {
    d.len() == 13 && d[..12].iter().all(|&v| v < 10) && check13_digit(&d[..12]) == d[12]
}

fn digits_str(d: &[u8]) -> String {
    d.iter()
        .map(|&v| if v == 10 { 'X' } else { (b'0' + v) as char })
        .collect()
}

/// ISBN-10 → ISBN-13 (`978` prefix, new check digit).
pub fn to_isbn13(d10: &[u8]) -> Vec<u8> {
    let mut d: Vec<u8> = vec![9, 7, 8];
    d.extend_from_slice(&d10[..9]);
    let c = check13_digit(&d);
    d.push(c);
    d
}

/// ISBN-13 (`978…`) → ISBN-10; `None` for `979…`.
pub fn to_isbn10(d13: &[u8]) -> Option<Vec<u8>> {
    if d13[..3] != [9, 7, 8] {
        return None;
    }
    let mut d = d13[3..12].to_vec();
    let s: u32 = d
        .iter()
        .enumerate()
        .map(|(i, &v)| (10 - i as u32) * v as u32)
        .sum();
    d.push(((11 - s % 11) % 11) as u8);
    Some(d)
}

/// Builds an [`Isbn`] from digits (`X` = 10) if the checksum is right.
fn make(d: &[u8], printed: &str) -> Option<Isbn> {
    let (d13, d10) = match d.len() {
        10 if check10(d) && d[..9].iter().all(|&v| v < 10) => (to_isbn13(d), Some(d.to_vec())),
        13 if check13(d) && (d[..3] == [9, 7, 8] || d[..3] == [9, 7, 9]) => {
            (d.to_vec(), to_isbn10(d))
        }
        _ => return None,
    };
    let printed = printed.trim_matches(|c: char| c == '-' || c.is_whitespace());
    let seps = printed
        .chars()
        .filter(|c| !c.is_ascii_alphanumeric())
        .count();
    let display = if d.len() == 13 && seps >= 3 {
        printed
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '-'
                }
            })
            .collect()
    } else {
        digits_str(&d13)
    };
    Some(Isbn {
        isbn13: digits_str(&d13),
        isbn10: d10.map(|v| digits_str(&v)),
        display,
    })
}

/// Valid ISBNs of a free-form field, in order, without duplicates (an ISBN-10 and its
/// ISBN-13 count once).
pub fn parse(raw: &str) -> Vec<Isbn> {
    let mut out: Vec<Isbn> = Vec::new();
    // tokens: runs of digits / X joined by dashes or single spaces
    let norm: String = raw
        .chars()
        .map(|c| match c {
            '‐' | '‑' | '‒' | '–' | '—' | '−' => '-',
            c if c.is_whitespace() => ' ',
            c => c,
        })
        .collect();
    let mut groups: Vec<&str> = Vec::new();
    for part in
        norm.split(|c: char| !(c.is_ascii_digit() || c == 'x' || c == 'X' || c == '-' || c == ' '))
    {
        groups.push(part);
    }
    for part in groups {
        // inside a part, words are separated by spaces; join words greedily into ISBNs
        let words: Vec<&str> = part.split(' ').filter(|w| !w.is_empty()).collect();
        let mut i = 0;
        while i < words.len() {
            let mut found = None;
            let mut digits: Vec<u8> = Vec::new();
            let mut j = i;
            while j < words.len() {
                for c in words[j].chars() {
                    match c {
                        '0'..='9' => digits.push(c as u8 - b'0'),
                        'x' | 'X' => digits.push(10),
                        _ => {}
                    }
                }
                j += 1;
                if digits.len() > 13 {
                    break;
                }
                if digits.len() == 10 || digits.len() == 13 {
                    // X only as the last ISBN-10 character
                    let x_ok = !digits[..digits.len() - 1].contains(&10)
                        && (digits.len() == 10 || digits[12] != 10);
                    if x_ok && let Some(isbn) = make(&digits, &words[i..j].join(" ")) {
                        found = Some((isbn, j));
                        // a 10-digit match may still grow into a 13-digit one: prefer 13
                        if digits.len() == 13 {
                            break;
                        }
                    }
                }
            }
            match found {
                Some((isbn, end)) => {
                    if !out.iter().any(|o| o.isbn13 == isbn.isbn13) {
                        out.push(isbn);
                    }
                    i = end;
                }
                None => i += 1,
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_forms() {
        let v = parse("978-5-699-12014-7");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].isbn13, "9785699120147");
        assert_eq!(v[0].isbn10.as_deref(), Some("5699120149"));
        assert_eq!(v[0].display, "978-5-699-12014-7");
        // ISBN-10 with prefix, shown as ISBN-13
        let v = parse("ISBN 5-17-012345-0");
        assert_eq!(v[0].isbn13, "9785170123452");
        assert_eq!(v[0].isbn10.as_deref(), Some("5170123450"));
        assert_eq!(v[0].display, "9785170123452");
        // X check digit, lower case
        let v = parse("isbn: 0-8044-2957-x");
        assert_eq!(v[0].isbn10.as_deref(), Some("080442957X"));
        assert_eq!(v[0].isbn13, "9780804429573");
        // 979 has no ISBN-10
        let v = parse("979-10-90636-07-1");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].isbn10, None);
    }

    #[test]
    fn several_and_duplicates() {
        let v = parse("5-17-012345-0; 978-5-17-012345-2, ISBN-13: 978 5 699 12014 7");
        let all: Vec<&str> = v.iter().map(|i| i.isbn13.as_str()).collect();
        assert_eq!(all, ["9785170123452", "9785699120147"]);
        // space-separated pair
        let v = parse("5170123450 9785699120147");
        assert_eq!(v.len(), 2);
        // en dashes
        let v = parse("978–5–699–12014–7");
        assert_eq!(v[0].isbn13, "9785699120147");
    }

    #[test]
    fn invalid_ones_are_dropped() {
        assert!(parse("978-5-699-12014-8").is_empty()); // wrong check digit
        assert!(parse("5-17-012345-4").is_empty());
        assert!(parse("12345").is_empty());
        assert!(parse("").is_empty());
        assert!(parse("нет").is_empty());
        assert!(parse("X-17-012345-3").is_empty());
        // a bad one next to a good one
        let v = parse("978-5-699-12014-8, 978-5-699-12014-7");
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn conversions() {
        let d10: Vec<u8> = "5170123450".bytes().map(|b| b - b'0').collect();
        let d13 = to_isbn13(&d10);
        assert_eq!(digits_str(&d13), "9785170123452");
        assert_eq!(digits_str(&to_isbn10(&d13).unwrap()), "5170123450");
    }
}
