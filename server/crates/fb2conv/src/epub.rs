//! EPUB 3 packaging: OPF, nav document, NCX (for EPUB 2 readers) and the zip container.

use std::io::{Cursor, Write};

use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::convert::Labels;
use crate::info::BookInfo;
use crate::xml::{esc, esc_text};
use crate::{Error, Result};

#[derive(Debug, Clone)]
pub(crate) struct XhtmlFile {
    pub name: String,
    pub title: String,
    pub body: String,
    pub body_class: &'static str,
    pub linear: bool,
    pub svg: bool,
    pub in_spine: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Resource {
    pub href: String,
    pub media_type: String,
    pub data: Vec<u8>,
    pub cover_image: bool,
    pub width: u32,
    pub height: u32,
    /// Font from [`crate::Assets`]: copied pre-compressed from its font archive.
    pub packed: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TocEntry {
    pub level: usize,
    pub title: String,
    pub file: usize,
    pub anchor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpineItem {
    File(usize),
    Nav,
}

pub(crate) struct Package<'a> {
    pub info: &'a BookInfo,
    pub lang: &'a str,
    pub identifier: String,
    pub files: Vec<XhtmlFile>,
    pub resources: Vec<Resource>,
    pub css: String,
    pub toc: Vec<TocEntry>,
    pub spine: Vec<SpineItem>,
    pub labels: Labels,
    pub cover_file: Option<usize>,
    pub body_start: usize,
    pub embed_fonts: bool,
    pub font_archive: &'a [u8],
}

/// 128-bit FNV-1a (deterministic across runs and platforms).
pub(crate) fn hash128(data: &[u8]) -> u128 {
    const PRIME: u128 = 0x0000000001000000000000000000013B;
    let mut h: u128 = 0x6c62272e07bb014262b821756295c58d;
    for &b in data {
        h ^= b as u128;
        h = h.wrapping_mul(PRIME);
    }
    h
}

fn uuid_from(h: u128) -> String {
    let mut b = h.to_be_bytes();
    b[6] = (b[6] & 0x0f) | 0x50; // version 5 style (name based)
    b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn is_uuid(s: &str) -> bool {
    let s = s.trim().trim_start_matches('{').trim_end_matches('}');
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `urn:uuid:…` from the FB2 document id when it is a UUID, otherwise derived from it (or from
/// the content hash when there is no id). Deterministic, so re-conversions keep the identifier.
pub(crate) fn identifier(fb2_id: &str, content_hash: u128) -> String {
    let id = fb2_id.trim();
    if is_uuid(id) {
        return format!(
            "urn:uuid:{}",
            id.trim_start_matches('{')
                .trim_end_matches('}')
                .to_ascii_lowercase()
        );
    }
    let h = if id.is_empty() {
        content_hash
    } else {
        hash128(id.as_bytes())
    };
    format!("urn:uuid:{}", uuid_from(h))
}

/// Current UTC time as `CCYY-MM-DDThh:mm:ssZ`.
pub(crate) fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // civil from days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

pub(crate) fn xhtml_doc(lang: &str, title: &str, body_class: &str, body: &str) -> String {
    let mut s = String::with_capacity(body.len() + 512);
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n");
    s.push_str(&format!(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"{lang}\" xml:lang=\"{lang}\">\n<head>\n<meta charset=\"utf-8\"/>\n<title>"
    ));
    esc_text(title, &mut s);
    s.push_str(
        "</title>\n<link rel=\"stylesheet\" type=\"text/css\" href=\"css/main.css\"/>\n</head>\n",
    );
    if body_class.is_empty() {
        s.push_str("<body>\n");
    } else {
        s.push_str(&format!("<body class=\"{body_class}\">\n"));
    }
    s.push_str(body);
    s.push_str("</body>\n</html>\n");
    s
}

struct TreeNode {
    entry: usize,
    children: Vec<usize>,
}

fn toc_tree(toc: &[TocEntry]) -> (Vec<TreeNode>, Vec<usize>) {
    let mut nodes: Vec<TreeNode> = Vec::with_capacity(toc.len());
    let mut roots = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (level, node)
    for (i, e) in toc.iter().enumerate() {
        while stack.last().is_some_and(|(l, _)| *l >= e.level) {
            stack.pop();
        }
        nodes.push(TreeNode {
            entry: i,
            children: Vec::new(),
        });
        match stack.last() {
            Some(&(_, p)) => nodes[p].children.push(i),
            None => roots.push(i),
        }
        stack.push((e.level, i));
    }
    (nodes, roots)
}

fn href(p: &Package, e: &TocEntry) -> String {
    let f = &p.files[e.file].name;
    match &e.anchor {
        Some(a) => format!("{f}#{a}"),
        None => f.clone(),
    }
}

fn nav_ol(p: &Package, nodes: &[TreeNode], list: &[usize], indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    out.push_str(&format!("{pad}<ol>\n"));
    for &n in list {
        let e = &p.toc[nodes[n].entry];
        out.push_str(&format!("{pad}  <li><a href=\"{}\">", esc(&href(p, e))));
        esc_text(&e.title, out);
        out.push_str("</a>");
        if !nodes[n].children.is_empty() {
            out.push('\n');
            nav_ol(p, nodes, &nodes[n].children, indent + 2, out);
            out.push_str(&format!("{pad}  "));
        }
        out.push_str("</li>\n");
    }
    out.push_str(&format!("{pad}</ol>\n"));
}

fn nav_doc(p: &Package) -> String {
    let (nodes, roots) = toc_tree(&p.toc);
    let mut body = String::new();
    body.push_str("<nav epub:type=\"toc\" id=\"toc\">\n<h1 class=\"titleblock h1\">");
    esc_text(p.labels.contents, &mut body);
    body.push_str("</h1>\n");
    nav_ol(p, &nodes, &roots, 0, &mut body);
    body.push_str("</nav>\n");
    let nav_in_spine = p.spine.contains(&SpineItem::Nav);
    body.push_str("<nav epub:type=\"landmarks\" id=\"landmarks\" hidden=\"hidden\">\n<ol>\n");
    if let Some(c) = p.cover_file {
        body.push_str(&format!(
            "  <li><a epub:type=\"cover\" href=\"{}\">",
            esc(&p.files[c].name)
        ));
        esc_text(p.labels.cover, &mut body);
        body.push_str("</a></li>\n");
    }
    if nav_in_spine {
        body.push_str("  <li><a epub:type=\"toc\" href=\"nav.xhtml#toc\">");
        esc_text(p.labels.contents, &mut body);
        body.push_str("</a></li>\n");
    }
    if let Some(f) = p.files.get(p.body_start) {
        body.push_str(&format!(
            "  <li><a epub:type=\"bodymatter\" href=\"{}\">",
            esc(&f.name)
        ));
        esc_text(p.labels.start, &mut body);
        body.push_str("</a></li>\n");
    }
    body.push_str("</ol>\n</nav>\n");
    xhtml_doc(p.lang, p.labels.contents, "", &body)
}

fn ncx_points(
    p: &Package,
    nodes: &[TreeNode],
    list: &[usize],
    order: &mut usize,
    depth: usize,
    max_depth: &mut usize,
    out: &mut String,
) {
    *max_depth = (*max_depth).max(depth);
    for &n in list {
        let e = &p.toc[nodes[n].entry];
        *order += 1;
        out.push_str(&format!(
            "<navPoint id=\"np{0}\" playOrder=\"{0}\"><navLabel><text>",
            *order
        ));
        esc_text(if e.title.is_empty() { "-" } else { &e.title }, out);
        out.push_str(&format!(
            "</text></navLabel><content src=\"{}\"/>\n",
            esc(&href(p, e))
        ));
        ncx_points(
            p,
            nodes,
            &nodes[n].children,
            order,
            depth + 1,
            max_depth,
            out,
        );
        out.push_str("</navPoint>\n");
    }
}

fn ncx(p: &Package) -> String {
    let (nodes, roots) = toc_tree(&p.toc);
    let mut points = String::new();
    let mut order = 0;
    let mut depth = 1;
    ncx_points(p, &nodes, &roots, &mut order, 1, &mut depth, &mut points);
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"");
    s.push_str(&format!(" xml:lang=\"{}\">\n<head>\n", p.lang));
    s.push_str(&format!(
        "<meta name=\"dtb:uid\" content=\"{}\"/>\n",
        esc(&p.identifier)
    ));
    s.push_str(&format!(
        "<meta name=\"dtb:depth\" content=\"{}\"/>\n",
        depth.saturating_sub(1).max(1)
    ));
    s.push_str("<meta name=\"dtb:totalPageCount\" content=\"0\"/>\n<meta name=\"dtb:maxPageNumber\" content=\"0\"/>\n</head>\n<docTitle><text>");
    esc_text(
        if p.info.title.is_empty() {
            "-"
        } else {
            &p.info.title
        },
        &mut s,
    );
    s.push_str("</text></docTitle>\n<navMap>\n");
    s.push_str(&points);
    s.push_str("</navMap>\n</ncx>\n");
    s
}

fn opf(p: &Package) -> String {
    let i = p.info;
    let mut m = String::new();
    m.push_str(&format!(
        "<dc:identifier id=\"bookid\">{}</dc:identifier>\n",
        esc(&p.identifier)
    ));
    m.push_str(&format!(
        "<dc:title id=\"title\">{}</dc:title>\n",
        esc(if i.title.is_empty() { "-" } else { &i.title })
    ));
    m.push_str(&format!("<dc:language>{}</dc:language>\n", esc(p.lang)));
    for (n, a) in i.authors.iter().enumerate() {
        let id = format!("creator{}", n + 1);
        m.push_str(&format!(
            "<dc:creator id=\"{id}\">{}</dc:creator>\n",
            esc(&a.natural_name())
        ));
        m.push_str(&format!(
            "<meta refines=\"#{id}\" property=\"role\" scheme=\"marc:relators\">aut</meta>\n"
        ));
        m.push_str(&format!(
            "<meta refines=\"#{id}\" property=\"file-as\">{}</meta>\n",
            esc(&a.file_as())
        ));
    }
    for (n, t) in i.translators.iter().enumerate() {
        let id = format!("translator{}", n + 1);
        m.push_str(&format!(
            "<dc:contributor id=\"{id}\">{}</dc:contributor>\n",
            esc(&t.natural_name())
        ));
        m.push_str(&format!(
            "<meta refines=\"#{id}\" property=\"role\" scheme=\"marc:relators\">trl</meta>\n"
        ));
    }
    if let Some(a) = &i.annotation {
        // plain text description
        let (root, _) = crate::dom::parse(
            &format!(
                "<d>{}</d>",
                a.replace("<br>", "<br/>").replace("</p>", "</p>\n")
            ),
            None,
        );
        let txt = root.text();
        let txt = txt
            .lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !txt.is_empty() {
            m.push_str(&format!("<dc:description>{}</dc:description>\n", esc(&txt)));
        }
    }
    for g in &i.genres {
        m.push_str(&format!("<dc:subject>{}</dc:subject>\n", esc(g)));
    }
    if !i.publisher.is_empty() {
        m.push_str(&format!(
            "<dc:publisher>{}</dc:publisher>\n",
            esc(&i.publisher)
        ));
    }
    let year = [&i.date, &i.year]
        .iter()
        .map(|d| d.trim().chars().take(4).collect::<String>())
        .find(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()));
    if let Some(y) = year {
        m.push_str(&format!("<dc:date>{y}</dc:date>\n"));
    }
    if !i.isbn.is_empty() {
        m.push_str(&format!(
            "<dc:identifier id=\"isbn\">urn:isbn:{}</dc:identifier>\n",
            esc(&i.isbn.replace([' ', '-'], ""))
        ));
    }
    m.push_str(&format!(
        "<meta property=\"dcterms:modified\">{}</meta>\n",
        now_utc()
    ));
    if let Some(s) = &i.series {
        m.push_str(&format!(
            "<meta property=\"belongs-to-collection\" id=\"series\">{}</meta>\n",
            esc(s)
        ));
        m.push_str("<meta refines=\"#series\" property=\"collection-type\">series</meta>\n");
        if let Some(n) = i.serno {
            m.push_str(&format!(
                "<meta refines=\"#series\" property=\"group-position\">{n}</meta>\n"
            ));
        }
        m.push_str(&format!(
            "<meta name=\"calibre:series\" content=\"{}\"/>\n",
            esc(s)
        ));
        if let Some(n) = i.serno {
            m.push_str(&format!(
                "<meta name=\"calibre:series_index\" content=\"{n}\"/>\n"
            ));
        }
    }
    let cover_idx = p.resources.iter().position(|r| r.cover_image);
    if cover_idx.is_some() {
        m.push_str("<meta name=\"cover\" content=\"cover-image\"/>\n");
    }

    let mut man = String::new();
    man.push_str("<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n");
    man.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n");
    man.push_str("<item id=\"css\" href=\"css/main.css\" media-type=\"text/css\"/>\n");
    for (n, f) in p.files.iter().enumerate() {
        let props = if f.svg { " properties=\"svg\"" } else { "" };
        man.push_str(&format!(
            "<item id=\"x{n}\" href=\"{}\" media-type=\"application/xhtml+xml\"{props}/>\n",
            esc(&f.name)
        ));
    }
    for (n, r) in p.resources.iter().enumerate() {
        if r.cover_image {
            man.push_str(&format!("<item id=\"cover-image\" href=\"{}\" media-type=\"{}\" properties=\"cover-image\"/>\n", esc(&r.href), r.media_type));
        } else {
            man.push_str(&format!(
                "<item id=\"r{n}\" href=\"{}\" media-type=\"{}\"/>\n",
                esc(&r.href),
                r.media_type
            ));
        }
    }
    let mut spine = String::new();
    for s in &p.spine {
        match s {
            SpineItem::Nav => spine.push_str("<itemref idref=\"nav\"/>\n"),
            SpineItem::File(n) => {
                let f = &p.files[*n];
                if f.in_spine {
                    let lin = if f.linear { "" } else { " linear=\"no\"" };
                    spine.push_str(&format!("<itemref idref=\"x{n}\"{lin}/>\n"));
                }
            }
        }
    }
    let mut guide = String::new();
    if let Some(c) = p.cover_file {
        guide.push_str(&format!(
            "<reference type=\"cover\" title=\"{}\" href=\"{}\"/>\n",
            esc(p.labels.cover),
            esc(&p.files[c].name)
        ));
    }
    if p.spine.contains(&SpineItem::Nav) {
        guide.push_str(&format!(
            "<reference type=\"toc\" title=\"{}\" href=\"nav.xhtml\"/>\n",
            esc(p.labels.contents)
        ));
    }
    if let Some(f) = p.files.get(p.body_start) {
        guide.push_str(&format!(
            "<reference type=\"text\" title=\"{}\" href=\"{}\"/>\n",
            esc(p.labels.start),
            esc(&f.name)
        ));
    }

    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"bookid\" xml:lang=\"{lang}\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:opf=\"http://www.idpf.org/2007/opf\">\n{m}</metadata>\n<manifest>\n{man}</manifest>\n<spine toc=\"ncx\">\n{spine}</spine>\n<guide>\n{guide}</guide>\n</package>\n",
        lang = p.lang
    )
}

const CONTAINER: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">\n<rootfiles>\n<rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\n</rootfiles>\n</container>\n";

const APPLE_OPTIONS: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<display_options>\n<platform name=\"*\">\n<option name=\"specified-fonts\">true</option>\n</platform>\n</display_options>\n";

pub(crate) fn zip_err(e: impl std::fmt::Display) -> Error {
    Error::Zip(e.to_string())
}

pub(crate) fn stored() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
}

pub(crate) fn deflated() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6))
}

pub(crate) fn write(p: &Package) -> Result<Vec<u8>> {
    let size_hint = p.files.iter().map(|f| f.body.len()).sum::<usize>() / 3
        + p.resources.iter().map(|r| r.data.len()).sum::<usize>();
    let mut zw = ZipWriter::new(Cursor::new(Vec::with_capacity(size_hint + 64 * 1024)));
    zw.start_file("mimetype", stored()).map_err(zip_err)?;
    zw.write_all(b"application/epub+zip")?;
    zw.start_file("META-INF/container.xml", deflated())
        .map_err(zip_err)?;
    zw.write_all(CONTAINER.as_bytes())?;
    if p.embed_fonts {
        zw.start_file("META-INF/com.apple.ibooks.display-options.xml", deflated())
            .map_err(zip_err)?;
        zw.write_all(APPLE_OPTIONS.as_bytes())?;
    }
    zw.start_file("OEBPS/content.opf", deflated())
        .map_err(zip_err)?;
    zw.write_all(opf(p).as_bytes())?;
    zw.start_file("OEBPS/toc.ncx", deflated())
        .map_err(zip_err)?;
    zw.write_all(ncx(p).as_bytes())?;
    zw.start_file("OEBPS/nav.xhtml", deflated())
        .map_err(zip_err)?;
    zw.write_all(nav_doc(p).as_bytes())?;
    zw.start_file("OEBPS/css/main.css", deflated())
        .map_err(zip_err)?;
    zw.write_all(p.css.as_bytes())?;
    for f in &p.files {
        let title = if f.title.is_empty() {
            &p.info.title
        } else {
            &f.title
        };
        let doc = xhtml_doc(p.lang, title, f.body_class, &f.body);
        zw.start_file(format!("OEBPS/{}", f.name), deflated())
            .map_err(zip_err)?;
        zw.write_all(doc.as_bytes())?;
    }
    let mut fonts = if p.resources.iter().any(|r| r.packed) {
        zip::ZipArchive::new(Cursor::new(p.font_archive)).ok()
    } else {
        None
    };
    for r in &p.resources {
        if r.packed
            && let Some(f) = fonts
                .as_mut()
                .and_then(|a| a.by_name(&format!("OEBPS/{}", r.href)).ok())
        {
            zw.raw_copy_file(f).map_err(zip_err)?;
            continue;
        }
        let opts = if r.media_type.starts_with("font/")
            || r.media_type == "image/svg+xml"
            || r.media_type == "image/bmp"
        {
            deflated()
        } else {
            stored()
        };
        zw.start_file(format!("OEBPS/{}", r.href), opts)
            .map_err(zip_err)?;
        zw.write_all(&r.data)?;
    }
    let cur = zw.finish().map_err(zip_err)?;
    Ok(cur.into_inner())
}
