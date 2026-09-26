//! FB2 → EPUB 3 conversion (port of `fb2mobi.cpp`, EPUB branch).

use std::collections::{HashMap, HashSet};

use crate::assets::Assets;
use crate::cover;
use crate::decode::{base64_decode, decode_xml, unzip_fb2};
use crate::dom::{self, Element, Node, collapse_ws};
use crate::epub::{self, Package, Resource, SpineItem, TocEntry, XhtmlFile};
use crate::hyph::Hyphenator;
use crate::images::{self, Kind, Prepared};
use crate::info::{BookInfo, parse_description};
use crate::meta::{self, BookMeta};
use crate::names::{NameFields, expand};
use crate::options::{ConvertOptions, CreateCover, Footnotes, Hyphenate, TocPlacement};
use crate::xml::{esc_attr, esc_text};
use crate::{Error, Result};

/// Start a new file before a section when the current one is bigger than this.
const SOFT_LIMIT: usize = 160 * 1024;
/// Split between blocks of a section when the current file is bigger than this.
const HARD_LIMIT: usize = 280 * 1024;

const LINK_START: char = '\u{1}';
const LINK_END: char = '\u{2}';

pub(crate) struct Labels {
    pub contents: &'static str,
    pub annotation: &'static str,
    pub notes: &'static str,
    pub cover: &'static str,
    pub start: &'static str,
    /// "Книга" in "Книга 3" on generated covers.
    pub book: &'static str,
}

pub(crate) fn labels(lang: &str) -> Labels {
    match lang.split('-').next().unwrap_or("") {
        "ru" | "be" => Labels {
            contents: "Содержание",
            annotation: "Аннотация",
            notes: "Примечания",
            cover: "Обложка",
            start: "Начало",
            book: "Книга",
        },
        "uk" => Labels {
            contents: "Зміст",
            annotation: "Анотація",
            notes: "Примітки",
            cover: "Обкладинка",
            start: "Початок",
            book: "Книга",
        },
        "de" => Labels {
            contents: "Inhalt",
            annotation: "Annotation",
            notes: "Anmerkungen",
            cover: "Umschlag",
            start: "Anfang",
            book: "Band",
        },
        _ => Labels {
            contents: "Contents",
            annotation: "Annotation",
            notes: "Notes",
            cover: "Cover",
            start: "Start",
            book: "Book",
        },
    }
}

/// A parsed FB2 document.
pub(crate) struct Doc {
    root: Element,
    pub info: BookInfo,
    cover_id: Option<String>,
    hash: u128,
}

impl Doc {
    pub fn load(bytes: &[u8]) -> Result<Doc> {
        let raw = unzip_fb2(bytes)?;
        let hash = epub::hash128(&raw);
        let src = decode_xml(&raw);
        let (root, _) = dom::parse(&src, None);
        let fb =
            dom::root_element(&root).ok_or_else(|| Error::Format("not an XML document".into()))?;
        if fb.name != "fictionbook" && fb.child("body").is_none() {
            return Err(Error::Format(format!(
                "not an FB2 document (root element <{}>)",
                fb.name
            )));
        }
        let (info, cover_id) = parse_description(fb.child("description"));
        Ok(Doc {
            root,
            info,
            cover_id,
            hash,
        })
    }

    fn fb(&self) -> &Element {
        dom::root_element(&self.root).expect("checked in load")
    }
}

struct Note<'a> {
    key: String,
    el: &'a Element,
    title: String,
    book: usize,
    backref: Option<String>,
    label: String,
}

struct BookCtx<'a> {
    prefix: String,
    binaries: HashMap<&'a str, &'a Element>,
}

pub(crate) struct Builder<'a> {
    opts: &'a ConvertOptions,
    assets: &'a Assets,
    labels: Labels,
    hyph: Option<&'a Hyphenator>,
    hyph_cache: crate::hyph::WordCache,
    files: Vec<XhtmlFile>,
    cur: String,
    cur_name: String,
    cur_title: Option<String>,
    cur_class: &'static str,
    part_counter: usize,
    toc: Vec<TocEntry>,
    ids: HashMap<String, (usize, String)>,
    used_ids: HashSet<String>,
    gen_counter: usize,
    resources: Vec<Resource>,
    res_names: HashSet<String>,
    image_map: HashMap<String, Option<usize>>,
    notes: Vec<Note<'a>>,
    note_index: HashMap<String, usize>,
    notes_titles: Vec<Option<String>>,
    books: Vec<BookCtx<'a>>,
    book: usize,
    level_base: usize,
    // rendering state
    no_hyph: u32,
    container_depth: u32,
    dropcap_pending: bool,
    at_block_start: bool,
    in_notes: bool,
    inline_note_depth: u32,
    heading_level_offset: usize,
}

fn is_uri(h: &str) -> bool {
    let l = h.to_ascii_lowercase();
    l.starts_with("http://")
        || l.starts_with("https://")
        || l.starts_with("mailto:")
        || l.starts_with("ftp://")
}

fn is_block_name(n: &str) -> bool {
    matches!(
        n,
        "p" | "v"
            | "stanza"
            | "poem"
            | "cite"
            | "subtitle"
            | "text-author"
            | "section"
            | "title"
            | "epigraph"
            | "annotation"
            | "table"
            | "tr"
            | "empty-line"
            | "date"
    )
}

/// Makes an XML-safe id from an arbitrary FB2 id.
fn sanitize_id(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !out.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        out.insert_str(0, "id");
    }
    out
}

fn sanitize_file_stem(s: &str) -> String {
    let stem = match s.rfind('.') {
        Some(i) if i > 0 => &s[..i],
        _ => s,
    };
    let mut out: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || !out.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        out.insert(0, 'i');
    }
    out.truncate(60);
    out
}

impl<'a> Builder<'a> {
    pub fn new(opts: &'a ConvertOptions, assets: &'a Assets, lang: String) -> Builder<'a> {
        let hyph = match opts.hyphenate {
            Hyphenate::None => None,
            _ => assets.hyphenator(&lang),
        };
        Builder {
            opts,
            assets,
            labels: labels(&lang),
            hyph,
            hyph_cache: Default::default(),
            files: Vec::new(),
            cur: String::new(),
            cur_name: String::new(),
            cur_title: None,
            cur_class: "",
            part_counter: 0,
            toc: Vec::new(),
            ids: HashMap::new(),
            used_ids: HashSet::new(),
            gen_counter: 0,
            resources: Vec::new(),
            res_names: HashSet::new(),
            image_map: HashMap::new(),
            notes: Vec::new(),
            note_index: HashMap::new(),
            notes_titles: Vec::new(),
            books: Vec::new(),
            book: 0,
            level_base: 0,
            no_hyph: 0,
            container_depth: 0,
            dropcap_pending: false,
            at_block_start: false,
            in_notes: false,
            inline_note_depth: 0,
            heading_level_offset: 0,
        }
    }

    // ---------------------------------------------------------------- files

    fn cur_index(&self) -> usize {
        self.files.len()
    }

    fn cur_has_content(&self) -> bool {
        !self.cur.trim().is_empty()
    }

    fn flush_file(&mut self) {
        if self.cur_has_content()
            || !self.cur_name.is_empty() && self.ids.values().any(|(f, _)| *f == self.files.len())
        {
            let body = std::mem::take(&mut self.cur);
            self.files.push(XhtmlFile {
                name: std::mem::take(&mut self.cur_name),
                title: self.cur_title.take().unwrap_or_default(),
                body,
                body_class: self.cur_class,
                linear: true,
                svg: false,
                in_spine: true,
            });
        } else {
            self.cur.clear();
            self.cur_title = None;
        }
    }

    fn start_file(&mut self, name: String, class: &'static str) {
        if !self.cur_name.is_empty() {
            self.flush_file();
        }
        self.cur_name = name;
        self.cur_class = class;
        self.cur_title = None;
    }

    fn new_part(&mut self) {
        if !self.cur_name.is_empty() && !self.cur_has_content() {
            return;
        }
        self.part_counter += 1;
        self.start_file(format!("part{:04}.xhtml", self.part_counter), "");
    }

    // ---------------------------------------------------------------- ids and links

    fn key(&self, fb2_id: &str) -> String {
        format!(
            "{}{}",
            self.books
                .get(self.book)
                .map(|b| b.prefix.as_str())
                .unwrap_or(""),
            fb2_id.trim()
        )
    }

    /// Registers an id for the element being written to the current file.
    fn register(&mut self, key: String, want: &str) -> Option<String> {
        if self.ids.contains_key(&key) || key.contains(LINK_END) {
            return None;
        }
        let base = sanitize_id(want);
        let mut id = base.clone();
        let mut n = 1;
        while self.used_ids.contains(&id) {
            n += 1;
            id = format!("{base}_{n}");
        }
        self.used_ids.insert(id.clone());
        self.ids.insert(key, (self.cur_index(), id.clone()));
        Some(id)
    }

    fn gen_id(&mut self, stem: &str) -> String {
        self.gen_counter += 1;
        let want = format!("{stem}{}", self.gen_counter);
        let key = format!("\u{3}{want}");
        self.register(key, &want).unwrap_or(want)
    }

    /// ` id="…"` for an FB2 element's id, or empty.
    fn id_attr(&mut self, e: &Element) -> String {
        match e.attr("id").map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => {
                let key = self.key(id);
                let want = format!(
                    "{}{}",
                    self.books
                        .get(self.book)
                        .map(|b| b.prefix.as_str())
                        .unwrap_or(""),
                    id
                );
                match self.register(key, &want) {
                    Some(h) => format!(" id=\"{h}\""),
                    None => String::new(),
                }
            }
            None => String::new(),
        }
    }

    fn link_marker(key: &str, out: &mut String) {
        out.push(LINK_START);
        out.push_str(key);
        out.push(LINK_END);
    }

    // ---------------------------------------------------------------- text

    fn text(&mut self, t: &str, out: &mut String) {
        let mut buf = String::with_capacity(t.len());
        let mut prev_ws = self.at_block_start || out.ends_with(' ');
        for c in t.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                if !prev_ws {
                    buf.push(' ');
                    prev_ws = true;
                }
            } else {
                buf.push(c);
                prev_ws = false;
            }
        }
        if buf.is_empty() {
            return;
        }
        if self.at_block_start {
            // dialogue dash: keep it on the line with the first word
            for d in ["— ", "– ", "- "] {
                if let Some(rest) = buf.strip_prefix(d) {
                    buf = format!("{}\u{a0}{}", &d[..d.len() - 1], rest);
                    break;
                }
            }
            self.at_block_start = false;
        }
        match self.hyph {
            Some(h) if self.no_hyph == 0 => {
                let mut hy = String::with_capacity(buf.len() + buf.len() / 4);
                h.hyphenate_cached(&buf, &mut hy, &mut self.hyph_cache);
                esc_text(&hy, out);
            }
            _ => esc_text(&buf, out),
        }
    }

    // ---------------------------------------------------------------- inline

    fn wrap(&mut self, e: &Element, open: &str, close: &str, out: &mut String) {
        let id = self.id_attr(e);
        let mut inner = String::new();
        self.inline(e, &mut inner);
        if inner.trim().is_empty() && id.is_empty() {
            out.push_str(&inner);
            return;
        }
        out.push('<');
        out.push_str(open);
        out.push_str(&id);
        out.push('>');
        out.push_str(&inner);
        out.push_str("</");
        out.push_str(close);
        out.push('>');
    }

    fn inline(&mut self, el: &Element, out: &mut String) {
        for c in &el.children {
            match c {
                Node::Text(t) => self.text(t, out),
                Node::Elem(e) => self.inline_elem(e, out),
            }
        }
    }

    fn inline_elem(&mut self, e: &Element, out: &mut String) {
        match e.name.as_str() {
            "strong" | "b" => self.wrap(e, "strong", "strong", out),
            "emphasis" | "i" | "em" => self.wrap(e, "em", "em", out),
            "strikethrough" => self.wrap(e, "span class=\"strike\"", "span", out),
            "sub" => self.wrap(e, "sub", "sub", out),
            "sup" => self.wrap(e, "sup", "sup", out),
            "code" => {
                self.no_hyph += 1;
                self.wrap(e, "code", "code", out);
                self.no_hyph -= 1;
            }
            "style" | "span" => self.wrap(e, "span", "span", out),
            "a" => self.link(e, out),
            "image" | "img" => self.inline_image(e, out),
            "empty-line" | "br" => out.push_str("<br/>"),
            "binary" | "stylesheet" | "description" => {}
            n => {
                if is_block_name(n) && !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                self.inline(e, out)
            }
        }
    }

    fn link(&mut self, e: &Element, out: &mut String) {
        let href = e.href().unwrap_or("").trim().to_string();
        if let Some(target) = href.strip_prefix('#') {
            let key = self.key(target);
            if let Some(&ni) = self.note_index.get(&key)
                && !self.in_notes
            {
                if self.opts.footnotes == Footnotes::Inline {
                    if self.inline_note_depth > 0 {
                        return;
                    }
                    self.inline_note_depth += 1;
                    let note_el = self.notes[ni].el;
                    let mut txt = String::new();
                    let saved = self.at_block_start;
                    self.at_block_start = true;
                    for c in note_el.elements().filter(|c| c.name != "title") {
                        let before = txt.len();
                        if before > 0 {
                            txt.push(' ');
                        }
                        self.inline(c, &mut txt);
                        if txt.len() == before + 1 {
                            txt.pop();
                        }
                    }
                    self.at_block_start = saved;
                    self.inline_note_depth -= 1;
                    let txt = txt.trim();
                    if !txt.is_empty() {
                        out.push_str(" <span class=\"inlinenote\">[");
                        out.push_str(txt);
                        out.push_str("]</span>");
                    }
                    return;
                }
                let ref_id = self.gen_id("ref");
                let ref_key = format!("\u{3}{ref_id}");
                let mut label = String::new();
                self.no_hyph += 1;
                self.inline(e, &mut label);
                self.no_hyph -= 1;
                let mut label = label.trim().to_string();
                if label.is_empty() {
                    label = "*".into();
                }
                let note = &mut self.notes[ni];
                if note.backref.is_none() {
                    note.backref = Some(ref_key);
                    note.label = e.clean_text();
                }
                out.push_str("<a class=\"anchor\" epub:type=\"noteref\" id=\"");
                out.push_str(&ref_id);
                out.push('"');
                Self::link_marker(&key, out);
                out.push('>');
                out.push_str(&label);
                out.push_str("</a>");
                return;
            }
            let id = self.id_attr(e);
            out.push_str("<a");
            out.push_str(&id);
            Self::link_marker(&key, out);
            out.push('>');
            self.inline(e, out);
            out.push_str("</a>");
        } else if is_uri(&href) {
            out.push_str("<a href=\"");
            esc_attr(&href, out);
            out.push_str("\">");
            self.inline(e, out);
            out.push_str("</a>");
        } else {
            self.wrap(e, "span", "span", out);
        }
    }

    // ---------------------------------------------------------------- images

    fn add_resource(&mut self, stem: &str, img: Prepared, cover: bool) -> usize {
        let mut name = format!("img/{stem}.{}", img.kind.ext());
        let mut n = 1;
        while self.res_names.contains(&name) {
            n += 1;
            name = format!("img/{stem}_{n}.{}", img.kind.ext());
        }
        self.res_names.insert(name.clone());
        self.resources.push(Resource {
            href: name,
            media_type: img.kind.mime().to_string(),
            data: img.data,
            cover_image: cover,
            width: img.width,
            height: img.height,
            packed: false,
        });
        self.resources.len() - 1
    }

    fn image_res(&mut self, e: &Element) -> Option<usize> {
        let href = e.href()?.trim();
        let id = href.strip_prefix('#')?;
        let key = self.key(id);
        if let Some(r) = self.image_map.get(&key) {
            return *r;
        }
        let bin = self.books[self.book].binaries.get(id.trim()).copied();
        let res = bin.and_then(|b| {
            let data = base64_decode(&b.text());
            let img = images::prepare(data)?;
            let stem = format!("{}{}", self.books[self.book].prefix, sanitize_file_stem(id));
            Some(self.add_resource(&stem, img, false))
        });
        self.image_map.insert(key, res);
        res
    }

    fn img_tag(&self, res: usize, alt: &str, class: Option<&str>, out: &mut String) {
        out.push_str("<img");
        if let Some(c) = class {
            out.push_str(" class=\"");
            out.push_str(c);
            out.push('"');
        }
        out.push_str(" src=\"");
        esc_attr(&self.resources[res].href, out);
        out.push_str("\" alt=\"");
        esc_attr(alt, out);
        out.push_str("\"/>");
    }

    fn inline_image(&mut self, e: &Element, out: &mut String) {
        let id = self.id_attr(e);
        match self.image_res(e) {
            Some(r) => {
                let alt = e.attr("alt").unwrap_or("").to_string();
                if id.is_empty() {
                    self.img_tag(r, &alt, Some("inlineimage"), out);
                } else {
                    out.push_str(&format!("<span{id}>"));
                    self.img_tag(r, &alt, Some("inlineimage"), out);
                    out.push_str("</span>");
                }
            }
            None if !id.is_empty() => out.push_str(&format!("<span{id}></span>")),
            None => {}
        }
    }

    fn block_image(&mut self, e: &Element, out: &mut String) {
        let id = self.id_attr(e);
        match self.image_res(e) {
            Some(r) => {
                out.push_str(&format!("<div class=\"image\"{id}>"));
                let alt = e.attr("alt").or(e.attr("title")).unwrap_or("").to_string();
                self.img_tag(r, &alt, None, out);
                if let Some(t) = e.attr("title").map(collapse_ws).filter(|t| !t.is_empty()) {
                    out.push_str("<p class=\"image-title\">");
                    esc_text(&t, out);
                    out.push_str("</p>");
                }
                out.push_str("</div>\n");
            }
            None if !id.is_empty() => out.push_str(&format!("<div{id}></div>\n")),
            None => {}
        }
    }

    // ---------------------------------------------------------------- blocks

    fn para(&mut self, e: &Element, tag: &str, class: &str, out: &mut String) {
        let id = self.id_attr(e);
        let mut inner = String::new();
        self.at_block_start = true;
        self.inline(e, &mut inner);
        self.at_block_start = false;
        while inner.ends_with(' ') {
            inner.pop();
        }
        if inner.trim().is_empty() && !inner.contains('<') {
            if !id.is_empty() {
                out.push_str(&format!("<div{id}></div>\n"));
            }
            return;
        }
        let mut class = class;
        if self.dropcap_pending && self.container_depth == 0 && tag == "p" && class.is_empty() {
            self.dropcap_pending = false;
            if apply_dropcap(&mut inner) {
                class = "dropcaps";
            }
        }
        out.push('<');
        out.push_str(tag);
        out.push_str(&id);
        if !class.is_empty() {
            out.push_str(" class=\"");
            out.push_str(class);
            out.push('"');
        }
        out.push('>');
        out.push_str(&inner);
        out.push_str("</");
        out.push_str(tag);
        out.push_str(">\n");
    }

    fn container(&mut self, e: &Element, open: &str, close: &str, out: &mut String) {
        let id = self.id_attr(e);
        let mut inner = String::new();
        self.container_depth += 1;
        self.blocks(e, &mut inner);
        self.container_depth -= 1;
        if inner.trim().is_empty() && id.is_empty() {
            return;
        }
        out.push('<');
        out.push_str(open);
        out.push_str(&id);
        out.push_str(">\n");
        out.push_str(&inner);
        out.push_str("</");
        out.push_str(close);
        out.push_str(">\n");
    }

    fn blocks(&mut self, el: &Element, out: &mut String) {
        let mut loose: Option<Element> = None;
        for c in &el.children {
            match c {
                Node::Text(t) if t.trim().is_empty() => {
                    if let Some(l) = loose.as_mut() {
                        l.children.push(c.clone());
                    }
                }
                Node::Elem(e)
                    if is_block_name(&e.name)
                        || matches!(e.name.as_str(), "image" | "code" | "div" | "blockquote")
                            && e.elements().any(|x| is_block_name(&x.name))
                        || e.name == "image" =>
                {
                    if let Some(l) = loose.take() {
                        self.para(&l, "p", "", out);
                    }
                    self.block(e, out);
                }
                _ => loose
                    .get_or_insert_with(Element::default)
                    .children
                    .push(c.clone()),
            }
        }
        if let Some(l) = loose.take() {
            self.para(&l, "p", "", out);
        }
    }

    fn title_lines(&mut self, t: &Element) -> String {
        let mut out = String::new();
        self.no_hyph += 1;
        let mut first = true;
        for c in &t.children {
            match c {
                Node::Elem(e) if e.name == "p" => {
                    let mut line = String::new();
                    self.at_block_start = true;
                    self.inline(e, &mut line);
                    self.at_block_start = false;
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    if !first {
                        out.push_str("<br/>");
                    }
                    out.push_str(line);
                    first = false;
                }
                Node::Elem(e) if e.name == "empty-line" => {
                    if !first {
                        out.push_str("<br/>");
                    }
                }
                Node::Elem(e) => {
                    let mut line = String::new();
                    self.inline_elem(e, &mut line);
                    out.push_str(line.trim());
                    first = first && line.trim().is_empty();
                }
                Node::Text(t) => {
                    if !t.trim().is_empty() {
                        let mut line = String::new();
                        self.text(t, &mut line);
                        out.push_str(line.trim());
                        first = false;
                    }
                }
            }
        }
        self.no_hyph -= 1;
        out
    }

    fn plain_title(&self, t: &Element) -> String {
        fn walk(b: &Builder, e: &Element, out: &mut String) {
            for c in &e.children {
                match c {
                    Node::Text(t) => out.push_str(t),
                    Node::Elem(x) => {
                        if x.name == "a"
                            && (x.attr("type") == Some("note")
                                || x.href()
                                    .and_then(|h| h.strip_prefix('#'))
                                    .is_some_and(|id| b.note_index.contains_key(&b.key(id))))
                        {
                            continue;
                        }
                        if x.name == "p" && !out.is_empty() {
                            out.push(' ');
                        }
                        walk(b, x, out);
                    }
                }
            }
        }
        let mut s = String::new();
        walk(self, t, &mut s);
        collapse_ws(&s)
    }

    /// Small title (poem, stanza, cite titles) rendered as a subtitle paragraph.
    fn minor_title(&mut self, t: &Element, class: &str, out: &mut String) {
        let id = self.id_attr(t);
        let lines = self.title_lines(t);
        if lines.is_empty() {
            return;
        }
        out.push_str(&format!("<p{id} class=\"{class}\">{lines}</p>\n"));
    }

    fn block(&mut self, e: &Element, out: &mut String) {
        match e.name.as_str() {
            "p" => self.para(e, "p", "", out),
            "subtitle" => {
                self.no_hyph += 1;
                self.para(e, "p", "subtitle", out);
                self.no_hyph -= 1;
            }
            "text-author" => self.para(e, "p", "text-author", out),
            "date" => self.para(e, "p", "date", out),
            "v" => self.para(e, "p", "v", out),
            "code" => {
                self.no_hyph += 1;
                let id = self.id_attr(e);
                let mut inner = String::new();
                self.inline(e, &mut inner);
                out.push_str(&format!(
                    "<p{id} class=\"code\"><code>{}</code></p>\n",
                    inner.trim()
                ));
                self.no_hyph -= 1;
            }
            "empty-line" => out.push_str("<p class=\"empty-line\">\u{a0}</p>\n"),
            "image" => self.block_image(e, out),
            "title" => self.minor_title(e, "subtitle", out),
            "epigraph" => self.container(e, "div class=\"epigraph\"", "div", out),
            "cite" => self.container(e, "blockquote class=\"cite\"", "blockquote", out),
            "annotation" => self.container(e, "div class=\"annotation\"", "div", out),
            "poem" => self.container(e, "div class=\"poem\"", "div", out),
            "stanza" => self.container(e, "div class=\"stanza\"", "div", out),
            "section" => {
                // nested section in a block context (notes, cites): flatten
                let id = self.id_attr(e);
                if !id.is_empty() {
                    out.push_str(&format!("<div{id}></div>\n"));
                }
                self.blocks(e, out);
            }
            "table" => self.table(e, out),
            "tr" => {
                // stray row outside a table
                let mut t = Element {
                    name: "table".into(),
                    ..Default::default()
                };
                t.children.push(Node::Elem(e.clone()));
                self.table(&t, out);
            }
            _ => {
                if e.elements().any(|x| is_block_name(&x.name)) {
                    self.blocks(e, out);
                } else {
                    self.para(e, "p", "", out);
                }
            }
        }
    }

    fn table(&mut self, e: &Element, out: &mut String) {
        let id = self.id_attr(e);
        let mut rows = String::new();
        self.container_depth += 1;
        for tr in e.elements().filter(|x| x.name == "tr") {
            let rid = self.id_attr(tr);
            let mut cells = String::new();
            for td in tr.elements().filter(|x| x.name == "td" || x.name == "th") {
                let cid = self.id_attr(td);
                let mut attrs = cid;
                for a in ["colspan", "rowspan"] {
                    if let Some(v) = td
                        .attr(a)
                        .and_then(|v| v.trim().parse::<u32>().ok())
                        .filter(|&v| v > 1 && v < 1000)
                    {
                        attrs.push_str(&format!(" {a}=\"{v}\""));
                    }
                }
                let mut style = String::new();
                if let Some(al) = td
                    .attr("align")
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|a| matches!(a.as_str(), "left" | "right" | "center" | "justify"))
                {
                    style.push_str(&format!("text-align: {al};"));
                }
                if let Some(va) = td
                    .attr("valign")
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|a| matches!(a.as_str(), "top" | "middle" | "bottom" | "baseline"))
                {
                    style.push_str(&format!("vertical-align: {va};"));
                }
                if !style.is_empty() {
                    attrs.push_str(&format!(" style=\"{style}\""));
                }
                let mut inner = String::new();
                self.at_block_start = true;
                self.inline(td, &mut inner);
                self.at_block_start = false;
                cells.push_str(&format!("<{0}{attrs}>{1}</{0}>", td.name, inner.trim()));
            }
            if !cells.is_empty() {
                rows.push_str(&format!("<tr{rid}>{cells}</tr>\n"));
            }
        }
        self.container_depth -= 1;
        if rows.is_empty() {
            return;
        }
        out.push_str(&format!("<table class=\"table\"{id}>\n{rows}</table>\n"));
    }

    // ---------------------------------------------------------------- structure

    fn heading(&mut self, t: &Element, depth: usize, sec_id: Option<&str>) {
        let level = depth + self.level_base;
        let h = (level + 1).min(6);
        let text = self.plain_title(t);
        let id = match sec_id {
            Some(sid) => {
                let key = self.key(sid);
                let want = format!("{}{}", self.books[self.book].prefix, sid);
                self.register(key, &want)
            }
            None => None,
        }
        .unwrap_or_else(|| self.gen_id("toc"));
        let lines = self.title_lines(t);
        let class = format!("titleblock h{}", depth.min(6));
        let pb = if depth > 1 && self.opts.break_after_chapter && self.cur_has_content() {
            " pb"
        } else {
            ""
        };
        self.cur.push_str(&format!(
            "<h{h} class=\"{class}{pb}\" id=\"{id}\">{lines}</h{h}>\n"
        ));
        if !text.is_empty() {
            self.toc.push(TocEntry {
                level,
                title: text.clone(),
                file: self.cur_index(),
                anchor: Some(id),
            });
            if self.cur_title.is_none() {
                self.cur_title = Some(text);
            }
        }
        self.dropcap_pending = self.opts.drop_caps && !self.in_notes;
    }

    fn anchor_div(&mut self, id: &str) {
        let key = self.key(id);
        let want = format!("{}{}", self.books[self.book].prefix, id);
        if let Some(h) = self.register(key, &want) {
            self.cur.push_str(&format!("<div id=\"{h}\"></div>\n"));
        }
    }

    fn block_to_cur(&mut self, e: &Element) {
        let mut buf = std::mem::take(&mut self.cur);
        self.block(e, &mut buf);
        self.cur = buf;
        if self.cur.len() > HARD_LIMIT {
            self.new_part();
        }
    }

    fn loose_to_cur(&mut self, nodes: &[Node]) {
        if nodes
            .iter()
            .all(|n| matches!(n, Node::Text(t) if t.trim().is_empty()))
        {
            return;
        }
        let el = Element {
            name: "p".into(),
            attrs: vec![],
            children: nodes.to_vec(),
        };
        self.block_to_cur(&el);
    }

    fn section(&mut self, s: &Element, depth: usize) {
        let title = s.child("title").filter(|t| t.has_text());
        let split = depth <= 1 || (self.opts.break_after_chapter && title.is_some() && depth <= 3);
        if (split && self.cur_has_content()) || self.cur.len() > SOFT_LIMIT {
            self.new_part();
        }
        let mut pending_id = s.attr("id").map(str::trim).filter(|s| !s.is_empty());
        let mut title_done = false;
        let mut loose: Vec<Node> = Vec::new();
        for c in &s.children {
            let Node::Elem(e) = c else {
                loose.push(c.clone());
                continue;
            };
            if !is_block_name(&e.name) && e.name != "image" {
                loose.push(c.clone());
                continue;
            }
            self.loose_to_cur(&std::mem::take(&mut loose));
            match e.name.as_str() {
                "title" if !title_done && title.is_some() => {
                    self.heading(e, depth, pending_id.take());
                    title_done = true;
                }
                "section" => {
                    if let Some(id) = pending_id.take() {
                        self.anchor_div(id);
                    }
                    self.section(e, depth + 1);
                }
                _ => {
                    if let Some(id) = pending_id.take() {
                        self.anchor_div(id);
                    }
                    self.block_to_cur(e);
                }
            }
        }
        self.loose_to_cur(&loose);
        if let Some(id) = pending_id {
            self.anchor_div(id);
        }
        self.dropcap_pending = false;
    }

    fn title_page(
        &mut self,
        info: &BookInfo,
        body: &Element,
        file_name: String,
        level: usize,
        annotation: Option<&Element>,
    ) {
        self.start_file(file_name, "");
        let id = self.gen_id("title");
        let body_title = body.child("title").filter(|t| t.has_text());
        let toc_title;
        if let Some(t) = body_title {
            let lines = self.title_lines(t);
            let h = (level + self.heading_level_offset).clamp(1, 6);
            self.cur.push_str(&format!(
                "<h{h} class=\"titleblock h0\" id=\"{id}\">{lines}</h{h}>\n"
            ));
            toc_title = if info.title.is_empty() {
                self.plain_title(t)
            } else {
                info.title.clone()
            };
        } else {
            let authors = info
                .authors
                .iter()
                .map(|a| a.natural_name())
                .collect::<Vec<_>>()
                .join(", ");
            let h = (level + self.heading_level_offset).clamp(1, 6);
            self.cur
                .push_str(&format!("<div class=\"titlepage\" id=\"{id}\">\n"));
            if !authors.is_empty() {
                self.cur.push_str("<p class=\"tp-author\">");
                esc_text(&authors, &mut self.cur);
                self.cur.push_str("</p>\n");
            }
            self.cur.push_str(&format!("<h{h} class=\"tp-title\">"));
            esc_text(
                if info.title.is_empty() {
                    "—"
                } else {
                    &info.title
                },
                &mut self.cur,
            );
            self.cur.push_str(&format!("</h{h}>\n"));
            if let Some(s) = &info.series {
                self.cur.push_str("<p class=\"tp-series\">");
                let s = match info.serno {
                    Some(n) => format!("{s} #{n}"),
                    None => s.clone(),
                };
                esc_text(&s, &mut self.cur);
                self.cur.push_str("</p>\n");
            }
            self.cur.push_str("</div>\n");
            toc_title = info.title.clone();
        }
        let toc_title = if toc_title.is_empty() {
            self.labels.start.to_string()
        } else {
            toc_title
        };
        self.cur_title = Some(toc_title.clone());
        self.toc.push(TocEntry {
            level,
            title: toc_title,
            file: self.cur_index(),
            anchor: None,
        });
        if let Some(a) = annotation {
            self.container_depth += 1;
            let mut buf = std::mem::take(&mut self.cur);
            self.container(a, "div class=\"annotation\"", "div", &mut buf);
            self.cur = buf;
            self.container_depth -= 1;
        }
    }

    /// Standalone annotation page (single-book mode, `annotation` option).
    fn annotation_page(&mut self, a: &Element) {
        self.start_file("annotation.xhtml".into(), "");
        let id = self.gen_id("annotation");
        let h = (1 + self.heading_level_offset).min(6);
        self.cur.push_str(&format!(
            "<div class=\"annotation\">\n<h{h} class=\"titleblock h1\" id=\"{id}\">"
        ));
        esc_text(self.labels.annotation, &mut self.cur);
        self.cur.push_str(&format!("</h{h}>\n"));
        let mut buf = std::mem::take(&mut self.cur);
        self.container_depth += 1;
        self.blocks(a, &mut buf);
        self.container_depth -= 1;
        self.cur = buf;
        self.cur.push_str("</div>\n");
        self.cur_title = Some(self.labels.annotation.to_string());
        let title = self.labels.annotation.to_string();
        self.toc.push(TocEntry {
            level: 1,
            title,
            file: self.cur_index(),
            anchor: None,
        });
        self.flush_file();
    }

    fn collect_notes(&mut self, el: &'a Element, book: usize) {
        for c in el.elements() {
            if c.name == "section"
                && let Some(id) = c.attr("id").map(str::trim).filter(|s| !s.is_empty())
            {
                let key = self.key(id);
                if self.note_index.contains_key(&key) {
                    continue;
                }
                let title = c.child("title").map(|t| t.clean_text()).unwrap_or_default();
                self.note_index.insert(key.clone(), self.notes.len());
                self.notes.push(Note {
                    key,
                    el: c,
                    title,
                    book,
                    backref: None,
                    label: String::new(),
                });
            } else if c.name == "section" || c.name == "body" {
                self.collect_notes(c, book);
            }
        }
    }

    /// Adds one FB2 book. In series mode each book becomes a part with its own title page.
    fn add_book(&mut self, doc: &'a Doc, joined: bool) {
        let fb = doc.fb();
        let book = self.books.len();
        let prefix = if joined {
            format!("b{}_", book + 1)
        } else {
            String::new()
        };
        let binaries = fb
            .children_named("binary")
            .filter_map(|b| b.attr("id").map(|id| (id.trim(), b)))
            .collect::<HashMap<_, _>>();
        self.books.push(BookCtx { prefix, binaries });
        self.book = book;
        let bodies: Vec<&'a Element> = fb.children_named("body").collect();
        type Bodies<'b> = Vec<(usize, &'b Element)>;
        let (main_bodies, note_bodies): (Bodies, Bodies) = bodies
            .iter()
            .copied()
            .enumerate()
            .partition(|(i, b)| *i == 0 || b.attr("name").is_none_or(|n| n.trim().is_empty()));
        self.notes_titles.push(None);
        for (_, nb) in &note_bodies {
            if self.notes_titles[book].is_none() {
                self.notes_titles[book] = nb
                    .child("title")
                    .map(|t| t.clean_text())
                    .filter(|t| !t.is_empty());
            }
            self.collect_notes(nb, book);
        }
        let annotation_el = fb
            .path(&["description", "title-info", "annotation"])
            .filter(|a| a.has_text());
        let level = 1;
        for (i, (_, body)) in main_bodies.iter().enumerate() {
            if i == 0 {
                let name = if joined {
                    format!("title{:02}.xhtml", book + 1)
                } else {
                    "title.xhtml".into()
                };
                let ann = if joined && self.opts.annotation {
                    annotation_el
                } else {
                    None
                };
                self.title_page(&doc.info, body, name, level, ann);
                self.level_base = if joined { 1 } else { 0 };
            } else {
                self.new_part();
                if let Some(t) = body.child("title").filter(|t| t.has_text()) {
                    self.heading(t, 1, None);
                }
            }
            let mut loose: Vec<Node> = Vec::new();
            for c in &body.children {
                match c {
                    Node::Elem(e) if e.name == "title" => {}
                    Node::Elem(e) if e.name == "section" => {
                        self.loose_to_cur(&std::mem::take(&mut loose));
                        self.section(e, 1);
                    }
                    Node::Elem(e) if is_block_name(&e.name) || e.name == "image" => {
                        self.loose_to_cur(&std::mem::take(&mut loose));
                        self.block_to_cur(e);
                    }
                    _ => loose.push(c.clone()),
                }
            }
            self.loose_to_cur(&loose);
        }
        self.flush_file();
        self.level_base = 0;
    }

    fn notes_files(&mut self, books_titles: &[String]) {
        if self.notes.is_empty() || self.opts.footnotes == Footnotes::Inline {
            return;
        }
        self.in_notes = true;
        let mut n_file = 1;
        self.start_file("notes.xhtml".into(), "");
        let title = self
            .notes_titles
            .iter()
            .flatten()
            .next()
            .cloned()
            .unwrap_or_else(|| self.labels.notes.to_string());
        let id = self.gen_id("notes");
        let h = (1 + self.heading_level_offset).min(6);
        self.cur
            .push_str(&format!("<h{h} class=\"titlenotes\" id=\"{id}\">"));
        esc_text(&title, &mut self.cur);
        self.cur.push_str(&format!("</h{h}>\n"));
        self.cur_title = Some(title.clone());
        self.toc.push(TocEntry {
            level: 1,
            title,
            file: self.cur_index(),
            anchor: Some(id),
        });
        let popup = self.opts.footnotes == Footnotes::Popup;
        let multi = self.books.len() > 1;
        let mut last_book = usize::MAX;
        for i in 0..self.notes.len() {
            let (key, el, ntitle, book, backref, label) = {
                let n = &self.notes[i];
                (
                    n.key.clone(),
                    n.el,
                    n.title.clone(),
                    n.book,
                    n.backref.clone(),
                    n.label.clone(),
                )
            };
            self.book = book;
            if self.cur.len() > SOFT_LIMIT {
                n_file += 1;
                self.start_file(format!("notes{n_file}.xhtml"), "");
            }
            if multi && book != last_book {
                let h2 = (h + 1).min(6);
                self.cur.push_str(&format!("<h{h2} class=\"titlenotes\">"));
                esc_text(
                    books_titles.get(book).map(String::as_str).unwrap_or(""),
                    &mut self.cur,
                );
                self.cur.push_str(&format!("</h{h2}>\n"));
            }
            last_book = book;
            let want = key.clone();
            let Some(hid) = self.register(key, &want) else {
                continue;
            };
            let shown = if !ntitle.is_empty() {
                ntitle
            } else if !label.is_empty() {
                label
            } else {
                "*".into()
            };
            let mut buf = std::mem::take(&mut self.cur);
            if popup {
                buf.push_str(&format!("<aside class=\"note\" epub:type=\"footnote\" id=\"{hid}\">\n<p class=\"note-title\">"));
                esc_text(&shown, &mut buf);
                buf.push_str("</p>\n");
            } else {
                buf.push_str(&format!("<div class=\"note\" epub:type=\"footnote\" id=\"{hid}\">\n<p class=\"note-title\">"));
                match &backref {
                    Some(r) => {
                        buf.push_str("<a");
                        Self::link_marker(r, &mut buf);
                        buf.push('>');
                        esc_text(&shown, &mut buf);
                        buf.push_str("</a>");
                    }
                    None => esc_text(&shown, &mut buf),
                }
                buf.push_str("</p>\n");
            }
            self.container_depth += 1;
            let body = Element {
                name: "section".into(),
                attrs: vec![],
                children: el
                    .children
                    .iter()
                    .filter(|c| !matches!(c, Node::Elem(e) if e.name == "title"))
                    .cloned()
                    .collect(),
            };
            self.blocks(&body, &mut buf);
            self.container_depth -= 1;
            buf.push_str(if popup { "</aside>\n" } else { "</div>\n" });
            self.cur = buf;
        }
        self.flush_file();
        self.in_notes = false;
    }

    /// Replaces link markers with real hrefs (or drops them when the target is unknown).
    fn resolve_links(&mut self) {
        let names: Vec<String> = self.files.iter().map(|f| f.name.clone()).collect();
        for f in &mut self.files {
            if !f.body.contains(LINK_START) {
                continue;
            }
            let mut out = String::with_capacity(f.body.len() + 256);
            let mut rest = f.body.as_str();
            while let Some(i) = rest.find(LINK_START) {
                out.push_str(&rest[..i]);
                let after = &rest[i + 1..];
                let Some(j) = after.find(LINK_END) else {
                    rest = after;
                    continue;
                };
                let key = &after[..j];
                if let Some((fi, id)) = self.ids.get(key) {
                    out.push_str(" href=\"");
                    if names[*fi] != f.name {
                        esc_attr(&names[*fi], &mut out);
                    }
                    out.push('#');
                    out.push_str(id);
                    out.push('"');
                }
                rest = &after[j + 1..];
            }
            out.push_str(rest);
            f.body = out;
        }
    }
}

fn apply_dropcap(html: &mut String) -> bool {
    let mut in_tag = false;
    for (i, c) in html.char_indices() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            c if c.is_whitespace() => {}
            c if c.is_alphabetic() => {
                let end = i + c.len_utf8();
                let letter = html[i..end].to_string();
                html.replace_range(i..end, &format!("<span class=\"dropcaps\">{letter}</span>"));
                return true;
            }
            _ => return false,
        }
    }
    false
}

fn name_fields(info: &BookInfo, lang: &str) -> NameFields {
    let a = info.authors.first().cloned().unwrap_or_default();
    NameFields {
        author_last: if a.last.is_empty() {
            a.nickname.clone()
        } else {
            a.last.clone()
        },
        author_first: a.first,
        author_middle: a.middle,
        series: info.series.clone(),
        serno: info.serno,
        title: info.title.clone(),
        lang: lang.to_string(),
        date: if info.date.is_empty() {
            info.year.clone()
        } else {
            info.date.clone()
        },
    }
}

fn prepare_cover(
    b: &mut Builder,
    doc: &Doc,
    info: &BookInfo,
    lang: &str,
    key_prefix: &str,
) -> Option<usize> {
    let opts = b.opts;
    let fb2_cover = doc
        .cover_id
        .as_deref()
        .and_then(|id| {
            let fb = doc.fb();
            let bin = fb
                .children_named("binary")
                .find(|x| x.attr("id").map(str::trim) == Some(id))?;
            images::prepare(base64_decode(&bin.text()))
        })
        .and_then(cover::checked);
    let label = opts
        .cover_label
        .as_deref()
        .map(|t| expand(t, &name_fields(info, lang)))
        .filter(|l| !l.trim().is_empty());
    let generate = |b: &Builder| {
        let authors = info
            .authors
            .iter()
            .map(|a| {
                let short = [a.first.trim(), a.last.trim()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" ");
                if short.is_empty() {
                    a.natural_name()
                } else {
                    short
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let bottom = label
            .clone()
            .unwrap_or_else(|| match (&info.series, info.serno) {
                (Some(s), Some(n)) => format!("{s}\n{} {n}", b.labels.book),
                (Some(s), None) => s.clone(),
                _ => String::new(),
            });
        let data = cover::generate_cover(b.assets, &authors, &info.title, &bottom);
        let (w, h) = images::dimensions(&data, Kind::Jpeg)
            .unwrap_or((cover::COVER_WIDTH, cover::COVER_HEIGHT));
        Prepared {
            data,
            kind: Kind::Jpeg,
            width: w,
            height: h,
        }
    };
    let (img, reusable) = match (opts.create_cover, fb2_cover) {
        (CreateCover::Always, _) | (CreateCover::Missing, None) => (generate(b), false),
        (_, Some(c)) => match &label {
            Some(l) => match cover::label_cover(b.assets, &c.data, l) {
                Some(data) => (
                    Prepared {
                        data,
                        kind: Kind::Jpeg,
                        width: c.width,
                        height: c.height,
                    },
                    false,
                ),
                None => (c, true),
            },
            None => (c, true),
        },
        (CreateCover::Never, None) => return None,
    };
    if img.data.is_empty() {
        return None;
    }
    let idx = b.add_resource("cover", img, true);
    if reusable && let Some(id) = &doc.cover_id {
        b.image_map.insert(format!("{key_prefix}{id}"), Some(idx));
    }
    Some(idx)
}

fn cover_page(b: &mut Builder, res: usize) -> XhtmlFile {
    let r = &b.resources[res];
    let (w, h) = (r.width.max(1), r.height.max(1));
    let body = format!(
        "<div class=\"cover\"><svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\" width=\"100%\" height=\"100%\" viewBox=\"0 0 {w} {h}\" preserveAspectRatio=\"xMidYMid meet\"><image width=\"{w}\" height=\"{h}\" xlink:href=\"{}\"/></svg></div>\n",
        r.href
    );
    XhtmlFile {
        name: "cover.xhtml".into(),
        title: b.labels.cover.to_string(),
        body,
        body_class: "cover",
        linear: true,
        svg: true,
        in_spine: true,
    }
}

fn build_css(opts: &ConvertOptions, assets: &Assets, fonts: &mut Vec<Resource>) -> String {
    let mut css = String::from(assets.default_css());
    css.push('\n');
    match opts.hyphenate {
        Hyphenate::Full => css.push_str("p, .poem, .cite, .epigraph {\n    -webkit-hyphens: auto;\n    -epub-hyphens: auto;\n    adobe-hyphenate: auto;\n    hyphens: auto;\n}\n"),
        _ => css.push_str("p, .poem, .cite, .epigraph {\n    -webkit-hyphens: manual;\n    -epub-hyphens: manual;\n    adobe-hyphenate: explicit;\n    hyphens: manual;\n}\n"),
    }
    if let Some(fam) = opts.font_family.as_deref().and_then(|f| assets.font(f)) {
        for face in &fam.faces {
            css.push_str(&format!(
                "@font-face {{\n    font-family: \"{}\";\n    font-weight: {};\n    font-style: {};\n    src: url(\"../fonts/{}\");\n}}\n",
                fam.name,
                if face.bold { "bold" } else { "normal" },
                if face.italic { "italic" } else { "normal" },
                face.file_name
            ));
            fonts.push(Resource {
                href: format!("fonts/{}", face.file_name),
                media_type: "font/ttf".into(),
                data: face.data.to_vec(),
                cover_image: false,
                width: 0,
                height: 0,
                packed: true,
            });
        }
        css.push_str(&format!(
            "body {{\n    font-family: \"{}\", serif;\n}}\n",
            fam.name
        ));
    }
    if opts.drop_caps {
        css.push_str("@font-face {\n    font-family: \"Sangha\";\n    src: url(\"../fonts/Sangha.ttf\");\n}\nspan.dropcaps {\n    font-family: \"Sangha\", serif;\n    font-weight: normal;\n}\n");
        fonts.push(Resource {
            href: "fonts/Sangha.ttf".into(),
            media_type: "font/ttf".into(),
            data: assets.dropcaps_font().to_vec(),
            cover_image: false,
            width: 0,
            height: 0,
            packed: true,
        });
    }
    if let Some(u) = opts.user_css.as_deref().filter(|u| !u.trim().is_empty()) {
        css.push_str("\n/* user CSS */\n");
        css.push_str(u);
        css.push('\n');
    }
    css
}

pub(crate) fn convert_docs(
    docs: &[Doc],
    opts: &ConvertOptions,
    assets: &Assets,
    series_title: Option<&str>,
    book_meta: &BookMeta,
) -> Result<Vec<u8>> {
    let first = docs
        .first()
        .ok_or_else(|| Error::Format("no books".into()))?;
    let joined = docs.len() > 1;
    let lang = meta::book_language(
        &first.info.lang,
        book_meta.lang.as_deref(),
        &first.info.title,
    );
    // metadata of the output
    let mut info = first.info.clone();
    if !joined {
        if let Some(s) = book_meta
            .series
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            // the catalog's series (what the user browses and sends by) wins; the FB2 number
            // is kept when it belongs to the same series
            let same = info
                .series
                .as_deref()
                .is_some_and(|f| f.trim().eq_ignore_ascii_case(s));
            info.serno = book_meta.serno.or(if same { info.serno } else { None });
            info.series = Some(s.to_string());
        }
        info.title = meta::clean_title(&info.title, info.serno);
    }
    if joined {
        let series = series_title
            .map(str::to_string)
            .or_else(|| first.info.series.clone())
            .unwrap_or_else(|| first.info.title.clone());
        info.title = series.clone();
        info.series = Some(series);
        info.serno = None;
        let mut authors = Vec::new();
        for d in docs {
            for a in &d.info.authors {
                if !authors.contains(a) {
                    authors.push(a.clone());
                }
            }
        }
        info.authors = authors;
    }
    let mut b = Builder::new(opts, assets, lang.clone());
    if joined {
        b.heading_level_offset = 0;
    }
    // prefix used by add_book for the first book
    let first_prefix = if joined { "b1_" } else { "" };
    let cover_res = prepare_cover(&mut b, first, &info, &lang, first_prefix);
    let mut pre_files = Vec::new();
    if let Some(r) = cover_res {
        pre_files.push(cover_page(&mut b, r));
    }
    // annotation page (single book)
    if !joined
        && opts.annotation
        && let Some(a) = first
            .fb()
            .path(&["description", "title-info", "annotation"])
            .filter(|a| a.has_text())
    {
        // render with book 0 context so ids/images resolve
        let fb = first.fb();
        let binaries = fb
            .children_named("binary")
            .filter_map(|x| x.attr("id").map(|id| (id.trim(), x)))
            .collect();
        b.books.push(BookCtx {
            prefix: String::new(),
            binaries,
        });
        b.book = 0;
        b.annotation_page(a);
        b.books.clear();
    }
    let n_pre = b.files.len();
    if joined {
        // series title page
        b.start_file("series.xhtml".into(), "");
        let id = b.gen_id("series");
        b.cur
            .push_str(&format!("<div class=\"titlepage\" id=\"{id}\">\n"));
        let authors = info
            .authors
            .iter()
            .map(|a| a.natural_name())
            .collect::<Vec<_>>()
            .join(", ");
        if !authors.is_empty() {
            b.cur.push_str("<p class=\"tp-author\">");
            esc_text(&authors, &mut b.cur);
            b.cur.push_str("</p>\n");
        }
        b.cur.push_str("<h1 class=\"tp-title\">");
        esc_text(&info.title, &mut b.cur);
        b.cur.push_str("</h1>\n</div>\n");
        b.cur_title = Some(info.title.clone());
        b.flush_file();
        b.heading_level_offset = 1;
    }
    let titles: Vec<String> = docs.iter().map(|d| d.info.title.clone()).collect();
    for d in docs {
        b.add_book(d, joined);
    }
    b.heading_level_offset = 0;
    b.notes_files(&titles);
    b.resolve_links();

    // assemble package
    let mut files = pre_files;
    let offset = files.len();
    let content_files = std::mem::take(&mut b.files);
    let toc: Vec<TocEntry> = b
        .toc
        .iter()
        .map(|t| TocEntry {
            file: t.file + offset,
            ..t.clone()
        })
        .collect();
    let body_start = offset + n_pre; // title page (or series page)
    files.extend(content_files);
    let mut fonts = Vec::new();
    let css = build_css(opts, assets, &mut fonts);
    let mut resources = std::mem::take(&mut b.resources);
    resources.extend(fonts);
    let identifier = match book_meta
        .book_key
        .as_deref()
        .filter(|k| !k.trim().is_empty())
    {
        Some(k) => meta::book_uuid(k),
        None => epub::identifier(
            &info.id,
            first.hash
                ^ docs
                    .iter()
                    .skip(1)
                    .fold(0u128, |a, d| a.rotate_left(7) ^ d.hash),
        ),
    };
    let title_sort = meta::title_sort(&info.title, &lang);
    let mut spine = Vec::new();
    let nav_pos = match opts.toc_placement {
        TocPlacement::Start => Some(body_start),
        TocPlacement::End => Some(files.len()),
        TocPlacement::None => None,
    };
    for i in 0..=files.len() {
        if nav_pos == Some(i) {
            spine.push(SpineItem::Nav);
        }
        if i < files.len() {
            spine.push(SpineItem::File(i));
        }
    }
    let pkg = Package {
        info: &info,
        lang: &lang,
        title_sort,
        identifier,
        files,
        resources,
        css,
        toc,
        spine,
        labels: labels(&lang),
        cover_file: cover_res.map(|_| 0),
        body_start,
        embed_fonts: opts.font_family.is_some() || opts.drop_caps,
        font_archive: assets.font_archive(),
    };
    epub::write(&pkg)
}

/// Converts one FB2 (plain or zipped, any encoding) to EPUB 3.
pub fn fb2_to_epub(bytes: &[u8], opts: &ConvertOptions, assets: &Assets) -> Result<Vec<u8>> {
    fb2_to_epub_with(bytes, opts, assets, &BookMeta::default())
}

/// [`fb2_to_epub`] with catalog metadata (stable identifier, fallback language, series).
pub fn fb2_to_epub_with(
    bytes: &[u8],
    opts: &ConvertOptions,
    assets: &Assets,
    meta: &BookMeta,
) -> Result<Vec<u8>> {
    let doc = Doc::load(bytes)?;
    convert_docs(std::slice::from_ref(&doc), opts, assets, None, meta)
}

/// Joins several FB2 books (e.g. a series, in reading order) into one EPUB: a series title page,
/// then each book as a part with its own title page and nested chapters. `title` defaults to
/// the series name of the first book.
pub fn join_to_epub(
    books: &[&[u8]],
    opts: &ConvertOptions,
    assets: &Assets,
    title: Option<&str>,
) -> Result<Vec<u8>> {
    join_to_epub_with(books, opts, assets, title, &BookMeta::default())
}

/// [`join_to_epub`] with catalog metadata: `meta.book_key` identifies the joined file (e.g.
/// all keys concatenated), `meta.lang` is the fallback language.
pub fn join_to_epub_with(
    books: &[&[u8]],
    opts: &ConvertOptions,
    assets: &Assets,
    title: Option<&str>,
    meta: &BookMeta,
) -> Result<Vec<u8>> {
    let docs = books
        .iter()
        .map(|b| Doc::load(b))
        .collect::<Result<Vec<_>>>()?;
    if docs.len() == 1 {
        return convert_docs(&docs, opts, assets, None, meta);
    }
    convert_docs(&docs, opts, assets, title, meta)
}
