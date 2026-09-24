//! Fast FB2 metadata reading (`read_info`).

use serde::{Deserialize, Serialize};

use crate::decode::{base64_decode, decode_xml, unzip_fb2};
use crate::dom::{self, Element, Node, collapse_ws};
use crate::images::{self, Kind};
use crate::{Error, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub first: String,
    pub middle: String,
    pub last: String,
    pub nickname: String,
}

impl Person {
    /// `Last First Middle` (catalog display order); the nickname when no name parts are set.
    pub fn display_name(&self) -> String {
        let s = join_non_empty(&[&self.last, &self.first, &self.middle]);
        if s.is_empty() { self.nickname.trim().to_string() } else { s }
    }

    /// `First Middle Last` (natural order, used in EPUB `dc:creator`).
    pub fn natural_name(&self) -> String {
        let s = join_non_empty(&[&self.first, &self.middle, &self.last]);
        if s.is_empty() { self.nickname.trim().to_string() } else { s }
    }

    /// `Last, First Middle` for `file-as`.
    pub fn file_as(&self) -> String {
        let rest = join_non_empty(&[&self.first, &self.middle]);
        match (self.last.trim().is_empty(), rest.is_empty()) {
            (false, false) => format!("{}, {}", self.last.trim(), rest),
            (false, true) => self.last.trim().to_string(),
            _ => self.natural_name(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.display_name().is_empty()
    }
}

fn join_non_empty(parts: &[&String]) -> String {
    parts.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverImage {
    pub data: Vec<u8>,
    /// `image/jpeg`, `image/png`, `image/gif`, `image/webp`, `image/bmp`
    pub mime: String,
}

/// Book metadata. Serialisable for caching; the cover bytes are not serialised.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookInfo {
    pub title: String,
    pub authors: Vec<Person>,
    pub translators: Vec<Person>,
    pub series: Option<String>,
    pub serno: Option<u32>,
    /// FB2 genre codes (`sf_social`, …) or EPUB subjects.
    pub genres: Vec<String>,
    pub lang: String,
    /// Sanitised HTML: `<p>`, `<em>`, `<strong>`, `<br>` only.
    pub annotation: Option<String>,
    pub keywords: String,
    /// `title-info/date` (free form, often a year).
    pub date: String,
    pub publisher: String,
    pub year: String,
    pub isbn: String,
    /// FB2 `document-info/id` or EPUB identifier.
    pub id: String,
    #[serde(skip)]
    pub cover: Option<CoverImage>,
}

fn person(el: &Element) -> Person {
    let t = |n: &str| el.child(n).map(|e| e.clean_text()).unwrap_or_default();
    Person { first: t("first-name"), middle: t("middle-name"), last: t("last-name"), nickname: t("nickname") }
}

pub(crate) fn parse_serno(s: &str) -> Option<u32> {
    let d: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse::<u32>().ok().filter(|&n| n > 0)
}

/// Metadata from `<description>`, plus the cover binary id (without `#`).
pub(crate) fn parse_description(desc: Option<&Element>) -> (BookInfo, Option<String>) {
    let mut info = BookInfo::default();
    let Some(desc) = desc else { return (info, None) };
    let mut cover_id = None;
    if let Some(ti) = desc.child("title-info") {
        info.title = ti.child("book-title").map(|e| e.clean_text()).unwrap_or_default();
        info.authors = ti.children_named("author").map(person).filter(|p| !p.is_empty()).collect();
        info.translators = ti.children_named("translator").map(person).filter(|p| !p.is_empty()).collect();
        info.genres = ti
            .children_named("genre")
            .map(|g| g.clean_text())
            .filter(|g| !g.is_empty())
            .fold(Vec::new(), |mut v, g| {
                if !v.contains(&g) {
                    v.push(g);
                }
                v
            });
        info.lang = ti.child("lang").map(|e| e.clean_text().to_lowercase()).unwrap_or_default();
        info.keywords = ti.child("keywords").map(|e| e.clean_text()).unwrap_or_default();
        info.date = ti
            .child("date")
            .map(|e| {
                let t = e.clean_text();
                if t.is_empty() { e.attr("value").unwrap_or_default().to_string() } else { t }
            })
            .unwrap_or_default();
        if let Some(seq) = ti.children_named("sequence").find(|s| s.attr("name").is_some_and(|n| !n.trim().is_empty())) {
            info.series = seq.attr("name").map(collapse_ws);
            info.serno = seq.attr("number").and_then(parse_serno);
        }
        info.annotation = ti.child("annotation").and_then(annotation_html);
        if let Some(cp) = ti.child("coverpage")
            && let Some(img) = cp.child("image")
            && let Some(h) = img.href()
        {
            let id = h.trim().trim_start_matches('#').to_string();
            if !id.is_empty() {
                cover_id = Some(id);
            }
        }
    }
    if let Some(pi) = desc.child("publish-info") {
        let t = |n: &str| pi.child(n).map(|e| e.clean_text()).unwrap_or_default();
        info.publisher = t("publisher");
        info.year = t("year");
        info.isbn = t("isbn");
        if info.series.is_none()
            && let Some(seq) = pi.children_named("sequence").find(|s| s.attr("name").is_some_and(|n| !n.trim().is_empty()))
        {
            info.series = seq.attr("name").map(collapse_ws);
            info.serno = seq.attr("number").and_then(parse_serno);
        }
    }
    if let Some(di) = desc.child("document-info") {
        info.id = di.child("id").map(|e| e.clean_text()).unwrap_or_default();
    }
    (info, cover_id)
}

// ---------------------------------------------------------------------------------------------
// annotation → sanitised HTML

fn esc(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if dom::is_xml_char(c) => out.push(c),
            _ => {}
        }
    }
}

fn inline_html(el: &Element, out: &mut String) {
    for c in &el.children {
        match c {
            Node::Text(t) => {
                // collapse runs of whitespace but keep single spaces
                let mut prev_ws = out.is_empty() || out.ends_with(' ') || out.ends_with("<br>");
                for ch in t.chars() {
                    if ch.is_whitespace() && ch != '\u{a0}' {
                        if !prev_ws {
                            out.push(' ');
                            prev_ws = true;
                        }
                    } else {
                        let mut b = [0u8; 4];
                        esc(ch.encode_utf8(&mut b), out);
                        prev_ws = false;
                    }
                }
            }
            Node::Elem(e) => match e.name.as_str() {
                "emphasis" | "em" | "i" => wrap(e, "em", out),
                "strong" | "b" => wrap(e, "strong", out),
                "br" | "empty-line" => out.push_str("<br>"),
                "p" | "div" | "v" | "subtitle" | "text-author" => {
                    if !out.is_empty() && !out.ends_with("<br>") {
                        out.push_str("<br>");
                    }
                    inline_html(e, out);
                }
                "image" | "img" | "binary" | "script" | "style" if e.name != "style" || !e.has_text() => {}
                _ => inline_html(e, out),
            },
        }
    }
}

fn wrap(e: &Element, tag: &str, out: &mut String) {
    let mut inner = String::new();
    inline_html(e, &mut inner);
    if inner.trim().is_empty() {
        out.push_str(&inner);
        return;
    }
    out.push('<');
    out.push_str(tag);
    out.push('>');
    out.push_str(&inner);
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

fn block_html(el: &Element, out: &mut String) {
    let mut loose = Element::default();
    let flush = |loose: &mut Element, out: &mut String| {
        if loose.has_text() {
            let mut p = String::new();
            inline_html(loose, &mut p);
            let p = p.trim();
            if !p.is_empty() {
                out.push_str("<p>");
                out.push_str(p);
                out.push_str("</p>");
            }
        }
        loose.children.clear();
    };
    for c in &el.children {
        match c {
            Node::Elem(e) => match e.name.as_str() {
                "p" | "subtitle" | "v" | "text-author" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    flush(&mut loose, out);
                    let mut p = String::new();
                    if e.name == "subtitle" || e.name.starts_with('h') && e.name.len() == 2 {
                        wrap(e, "strong", &mut p);
                    } else {
                        inline_html(e, &mut p);
                    }
                    let p = p.trim().trim_end_matches("<br>").trim();
                    if !p.is_empty() {
                        out.push_str("<p>");
                        out.push_str(p);
                        out.push_str("</p>");
                    }
                }
                "empty-line" | "br" if loose.children.is_empty() => {}
                "poem" | "stanza" | "cite" | "epigraph" | "div" | "section" | "blockquote" | "ul" | "ol" | "table"
                | "tr" | "td" | "th" | "annotation" | "body" => {
                    flush(&mut loose, out);
                    block_html(e, out);
                }
                "title" => {
                    flush(&mut loose, out);
                    let mut p = String::new();
                    wrap(e, "strong", &mut p);
                    if !p.trim().is_empty() {
                        out.push_str("<p>");
                        out.push_str(p.trim());
                        out.push_str("</p>");
                    }
                }
                _ => loose.children.push(c.clone()),
            },
            Node::Text(_) => loose.children.push(c.clone()),
        }
    }
    flush(&mut loose, out);
}

/// Converts an FB2 `<annotation>` (or any element with FB2/HTML-ish content) to sanitised HTML
/// limited to `<p>`, `<em>`, `<strong>`, `<br>`. Returns `None` when there is no text.
pub fn annotation_html(el: &Element) -> Option<String> {
    let mut out = String::new();
    block_html(el, &mut out);
    if out.is_empty() { None } else { Some(out) }
}

/// Sanitises an HTML fragment (e.g. EPUB `dc:description`) the same way.
pub fn sanitize_html(html: &str) -> Option<String> {
    let src = format!("<div>{html}</div>");
    let (root, _) = dom::parse(&src, None);
    annotation_html(&root)
}

// ---------------------------------------------------------------------------------------------

/// Locates `<binary id="…">` content in the raw text without building a DOM.
pub(crate) fn find_binary<'a>(src: &'a str, id: &str) -> Option<(&'a str, Option<String>)> {
    let mut pos = 0;
    while let Some(rel) = src[pos..].find("binary") {
        let at = pos + rel;
        pos = at + 6;
        let before = src[..at].chars().next_back();
        if !matches!(before, Some('<') | Some(':')) {
            continue;
        }
        let tag_end = at + src[at..].find('>')?;
        let tag = &src[at - 1..=tag_end];
        if !(tag.contains(id)) {
            continue;
        }
        let (el, _) = dom::parse(tag, None);
        let Some(el) = dom::root_element(&el) else { continue };
        if el.attr("id").map(str::trim) != Some(id) {
            continue;
        }
        let body_start = tag_end + 1;
        let body_end = src[body_start..].find("</").map(|i| body_start + i).unwrap_or(src.len());
        return Some((&src[body_start..body_end], el.attr("content-type").map(str::to_string)));
    }
    None
}

pub(crate) fn cover_from_base64(b64: &str) -> Option<CoverImage> {
    let data = base64_decode(b64);
    let kind = images::sniff(&data);
    match kind {
        Kind::Jpeg | Kind::Png | Kind::Gif | Kind::Webp | Kind::Bmp => {
            images::dimensions(&data, kind)?;
            Some(CoverImage { data, mime: kind.mime().to_string() })
        }
        _ => None,
    }
}

/// Reads FB2 metadata, annotation and cover. Accepts plain or zipped FB2 in any common
/// encoding. Only `<description>` is parsed into a tree; the cover binary is located by a
/// text search, so this is fast even for large books.
pub fn read_info(bytes: &[u8]) -> Result<BookInfo> {
    let raw = unzip_fb2(bytes)?;
    let src = decode_xml(&raw);
    let (root, pos) = dom::parse(&src, Some("description"));
    let fb = dom::root_element(&root).ok_or_else(|| Error::Format("not an XML document".into()))?;
    if fb.name != "fictionbook" && fb.name != "FictionBook" && fb.child("description").is_none() {
        return Err(Error::Format(format!("not an FB2 document (root element <{}>)", fb.name)));
    }
    let (mut info, cover_id) = parse_description(fb.child("description"));
    if let Some(id) = cover_id {
        // If parsing ran to EOF (no </description>), binaries are in the tree already.
        let from_tree = fb
            .children_named("binary")
            .find(|b| b.attr("id").map(str::trim) == Some(id.as_str()))
            .and_then(|b| cover_from_base64(&b.text()));
        info.cover = from_tree.or_else(|| find_binary(&src[pos.min(src.len())..], &id).and_then(|(b64, _)| cover_from_base64(b64)));
    }
    Ok(info)
}
