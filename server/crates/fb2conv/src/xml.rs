//! Escaping helpers.

use crate::dom::is_xml_char;

pub fn esc_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if is_xml_char(c) => out.push(c),
            _ => {}
        }
    }
}

pub fn esc_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if is_xml_char(c) => out.push(c),
            _ => {}
        }
    }
}

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    esc_attr(s, &mut o);
    o
}
