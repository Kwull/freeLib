//! A tiny, tolerant XML DOM built on top of quick-xml.
//!
//! FB2 files in the wild are frequently broken: unknown HTML entities (`&nbsp;`), stray `&`,
//! mismatched or unclosed tags, unquoted attributes. The parser here never fails: it keeps what
//! it could read, closes open elements at EOF, resolves HTML entities and drops namespace
//! prefixes from element and attribute names (`l:href` → `href`, `fb:p` → `p`).

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

#[derive(Debug, Clone, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Elem(Element),
    Text(String),
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Elem(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.name == name)
    }

    /// Follows a path of child names, e.g. `["title-info", "book-title"]`.
    pub fn path(&self, path: &[&str]) -> Option<&Element> {
        let mut cur = self;
        for p in path {
            cur = cur.child(p)?;
        }
        Some(cur)
    }

    /// Concatenated text of all descendants.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s
    }

    pub fn collect_text(&self, out: &mut String) {
        for c in &self.children {
            match c {
                Node::Text(t) => out.push_str(t),
                Node::Elem(e) => e.collect_text(out),
            }
        }
    }

    /// Text with whitespace collapsed and trimmed.
    pub fn clean_text(&self) -> String {
        collapse_ws(&self.text())
    }

    /// `href` attribute regardless of namespace prefix (`l:href`, `xlink:href`, `href`).
    pub fn href(&self) -> Option<&str> {
        self.attr("href")
    }

    pub fn has_text(&self) -> bool {
        self.children.iter().any(|c| match c {
            Node::Text(t) => !t.trim().is_empty(),
            Node::Elem(e) => e.has_text(),
        })
    }
}

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

fn local(name: &str) -> &str {
    match name.rfind(':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

fn norm_name(name: &str) -> String {
    let l = local(name);
    if l.bytes().any(|b| b.is_ascii_uppercase()) {
        l.to_ascii_lowercase()
    } else {
        l.to_string()
    }
}

/// Resolves a named or numeric entity. Returns `None` for unknown names.
pub fn resolve_entity(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let v = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse::<u32>().ok()?
        };
        return char::from_u32(v).filter(|&c| is_xml_char(c));
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "shy" => '\u{ad}',
        "laquo" => '«',
        "raquo" => '»',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "mdash" => '—',
        "ndash" => '–',
        "hellip" => '…',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "deg" => '°',
        "plusmn" => '±',
        "times" => '×',
        "divide" => '÷',
        "middot" => '·',
        "bull" => '•',
        "sect" => '§',
        "para" => '¶',
        "euro" => '€',
        "pound" => '£',
        "cent" => '¢',
        "yen" => '¥',
        "iexcl" => '¡',
        "iquest" => '¿',
        "frac12" => '½',
        "frac14" => '¼',
        "frac34" => '¾',
        "sup1" => '¹',
        "sup2" => '²',
        "sup3" => '³',
        "prime" => '′',
        "Prime" => '″',
        "thinsp" => '\u{2009}',
        "ensp" => '\u{2002}',
        "emsp" => '\u{2003}',
        "zwnj" => '\u{200c}',
        "zwj" => '\u{200d}',
        "lrm" => '\u{200e}',
        "rlm" => '\u{200f}',
        "numero" => '№',
        "larr" => '←',
        "rarr" => '→',
        "uarr" => '↑',
        "darr" => '↓',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "Auml" => 'Ä',
        "Ouml" => 'Ö',
        "Uuml" => 'Ü',
        "szlig" => 'ß',
        "eacute" => 'é',
        "egrave" => 'è',
        "agrave" => 'à',
        "ccedil" => 'ç',
        _ => return None,
    })
}

pub fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

/// Unescapes attribute/text content that may still contain entity references.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c.is_whitespace() || c == '<');
        match end {
            Some(e) if rest.as_bytes()[e + 1] == b';' => {
                let name = &rest[1..e + 1];
                if let Some(c) = resolve_entity(name) {
                    out.push(c);
                } else {
                    out.push_str(&rest[..e + 2]);
                }
                rest = &rest[e + 2..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn make_element(e: &BytesStart) -> Element {
    let name = norm_name(e.name().as_ref());
    let mut attrs = Vec::new();
    for a in e.html_attributes().with_checks(false).flatten() {
        let key = norm_name(a.key.as_ref());
        if key.is_empty() || key == "xmlns" || a.key.as_ref().starts_with("xmlns:") {
            continue;
        }
        let v = unescape(&a.value);
        if !attrs.iter().any(|(k, _): &(String, String)| *k == key) {
            attrs.push((key, v));
        }
    }
    Element { name, attrs, children: Vec::new() }
}

/// Parses `src` into a synthetic root element whose children are the top-level nodes.
///
/// If `stop_after` is given, parsing stops after the end tag of the first element with that
/// (local) name — used to read only the `<description>` of large FB2 files. The byte offset in
/// `src` where parsing stopped is returned as well.
pub fn parse(src: &str, stop_after: Option<&str>) -> (Element, usize) {
    let mut reader = Reader::from_str(src);
    {
        let cfg = reader.config_mut();
        cfg.allow_dangling_amp = true;
        cfg.allow_unmatched_ends = true;
        cfg.check_end_names = false;
        cfg.check_comments = false;
        cfg.expand_empty_elements = false;
    }
    let mut stack: Vec<Element> = vec![Element { name: String::new(), ..Default::default() }];
    let mut errors = 0;
    loop {
        let ev = match reader.read_event() {
            Ok(ev) => ev,
            Err(_) => {
                errors += 1;
                if errors > 1000 {
                    break;
                }
                continue;
            }
        };
        match ev {
            Event::Start(e) => {
                stack.push(make_element(&e));
            }
            Event::Empty(e) => {
                let el = make_element(&e);
                stack.last_mut().unwrap().children.push(Node::Elem(el));
            }
            Event::End(e) => {
                let name = norm_name(e.name().as_ref());
                // Find the matching open element; ignore stray end tags.
                if let Some(pos) = stack.iter().rposition(|el| el.name == name)
                    && pos > 0
                {
                    while stack.len() > pos {
                        let el = stack.pop().unwrap();
                        stack.last_mut().unwrap().children.push(Node::Elem(el));
                    }
                    if stop_after == Some(name.as_str()) {
                        let pos = reader.buffer_position() as usize;
                        return (close_all(stack), pos);
                    }
                }
            }
            Event::Text(t) => {
                let s = t.xml10_content();
                push_text(stack.last_mut().unwrap(), &s);
            }
            Event::CData(t) => {
                let s: &str = &t;
                push_text(stack.last_mut().unwrap(), s);
            }
            Event::GeneralRef(r) => {
                let name: &str = &r;
                let el = stack.last_mut().unwrap();
                match resolve_entity(name) {
                    Some(c) => {
                        let mut buf = [0u8; 4];
                        push_text(el, c.encode_utf8(&mut buf));
                    }
                    None => push_text(el, &format!("&{name};")),
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let pos = reader.buffer_position() as usize;
    (close_all(stack), pos)
}

fn close_all(mut stack: Vec<Element>) -> Element {
    while stack.len() > 1 {
        let el = stack.pop().unwrap();
        stack.last_mut().unwrap().children.push(Node::Elem(el));
    }
    stack.pop().unwrap()
}

fn push_text(el: &mut Element, s: &str) {
    if s.is_empty() {
        return;
    }
    let clean: std::borrow::Cow<str> = if s.chars().all(is_xml_char) {
        s.into()
    } else {
        s.chars().filter(|&c| is_xml_char(c)).collect::<String>().into()
    };
    if let Some(Node::Text(t)) = el.children.last_mut() {
        t.push_str(&clean);
    } else {
        el.children.push(Node::Text(clean.into_owned()));
    }
}

/// Returns the document element (first element child of the synthetic root).
pub fn root_element(root: &Element) -> Option<&Element> {
    root.elements().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerant() {
        let (root, _) = parse(
            "<?xml version=\"1.0\"?><a x=1 l:href='#y'><b>one &nbsp;&amp; &foo; & two</c></b><p>unclosed",
            None,
        );
        let a = root_element(&root).unwrap();
        assert_eq!(a.attr("x"), Some("1"));
        assert_eq!(a.href(), Some("#y"));
        let b = a.child("b").unwrap();
        assert_eq!(b.text(), "one \u{a0}& &foo; & two");
        assert_eq!(a.child("p").unwrap().text(), "unclosed");
    }

    #[test]
    fn stop_after() {
        let src = "<r><description><t>x</t></description><body>zzz</body></r>";
        let (root, pos) = parse(src, Some("description"));
        let r = root_element(&root).unwrap();
        assert_eq!(r.child("description").unwrap().text(), "x");
        assert!(src[pos..].starts_with("<body>"));
    }
}
