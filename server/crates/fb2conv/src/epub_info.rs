//! Metadata of EPUB inputs (`read_info_epub`).

use std::io::Cursor;

use crate::decode::decode_xml;
use crate::dom::{self, Element, collapse_ws};
use crate::images::{self, Kind};
use crate::info::{BookInfo, CoverImage, Person, parse_serno, sanitize_html};
use crate::{Error, Result};

type Zip<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

const MAX_ENTRY: u64 = 64 * 1024 * 1024;

pub(crate) fn read_entry(zip: &mut Zip, name: &str) -> Option<Vec<u8>> {
    let idx = zip.index_for_name(name).or_else(|| {
        // case-insensitive fallback
        let lower = name.to_ascii_lowercase();
        (0..zip.len()).find(|&i| {
            zip.name_for_index(i)
                .is_some_and(|n| n.to_ascii_lowercase() == lower)
        })
    })?;
    let f = zip.by_index(idx).ok()?;
    let hint = f.size();
    // entries past the limit (zip bombs) are treated as missing
    crate::limit::read_limited(f, MAX_ENTRY, hint).ok()
}

fn parse_xml(bytes: &[u8]) -> Element {
    let src = decode_xml(bytes);
    dom::parse(&src, None).0
}

fn find_desc<'a>(el: &'a Element, name: &str, out: &mut Vec<&'a Element>) {
    for c in el.elements() {
        if c.name == name {
            out.push(c);
        }
        find_desc(c, name, out);
    }
}

pub(crate) fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Resolves `href` relative to the directory of `base` (a path inside the zip).
pub(crate) fn resolve_path(base: &str, href: &str) -> String {
    let href = percent_decode(href.split('#').next().unwrap_or(""));
    let mut parts: Vec<&str> = match base.rfind('/') {
        Some(i) => base[..i].split('/').filter(|s| !s.is_empty()).collect(),
        None => Vec::new(),
    };
    for seg in href.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn person_from_name(name: &str, file_as: Option<&str>) -> Person {
    if let Some(fa) = file_as
        && let Some((last, rest)) = fa.split_once(',')
    {
        let mut it = rest.split_whitespace();
        let first = it.next().unwrap_or("").to_string();
        let middle = it.collect::<Vec<_>>().join(" ");
        return Person {
            first,
            middle,
            last: last.trim().to_string(),
            nickname: String::new(),
        };
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    match words.len() {
        0 => Person::default(),
        1 => Person {
            last: words[0].to_string(),
            ..Default::default()
        },
        2 => Person {
            first: words[0].to_string(),
            last: words[1].to_string(),
            ..Default::default()
        },
        n => Person {
            first: words[0].to_string(),
            middle: words[1..n - 1].join(" "),
            last: words[n - 1].to_string(),
            nickname: String::new(),
        },
    }
}

/// Reads EPUB metadata (OPF `dc:*`, Calibre / EPUB 3 series) and the cover image.
pub fn read_info_epub(bytes: &[u8]) -> Result<BookInfo> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Zip(e.to_string()))?;
    let container = read_entry(&mut zip, "META-INF/container.xml")
        .ok_or_else(|| Error::Format("EPUB without META-INF/container.xml".into()))?;
    let croot = parse_xml(&container);
    let mut rootfiles = Vec::new();
    find_desc(&croot, "rootfile", &mut rootfiles);
    let opf_path = rootfiles
        .iter()
        .find(|r| {
            r.attr("media-type")
                .is_none_or(|m| m == "application/oebps-package+xml")
        })
        .and_then(|r| r.attr("full-path"))
        .ok_or_else(|| Error::Format("EPUB container without rootfile".into()))?
        .to_string();
    let opf = read_entry(&mut zip, &opf_path)
        .ok_or_else(|| Error::Format(format!("missing OPF {opf_path}")))?;
    let oroot = parse_xml(&opf);
    let package = dom::root_element(&oroot).ok_or_else(|| Error::Format("empty OPF".into()))?;
    let meta = package
        .child("metadata")
        .ok_or_else(|| Error::Format("OPF without metadata".into()))?;
    // Some OPFs nest dc-metadata (OPF 1.x); flatten
    let mut md: Vec<&Element> = meta.elements().collect();
    if let Some(dcm) = meta.child("dc-metadata") {
        md.extend(dcm.elements());
    }
    if let Some(xm) = meta.child("x-metadata") {
        md.extend(xm.elements());
    }

    let mut info = BookInfo::default();
    let refines = |id: Option<&str>, prop: &str| -> Option<String> {
        let id = id?;
        md.iter()
            .find(|m| {
                m.name == "meta"
                    && m.attr("refines") == Some(&format!("#{id}"))
                    && m.attr("property") == Some(prop)
            })
            .map(|m| m.clean_text())
    };
    for m in &md {
        match m.name.as_str() {
            "title" if info.title.is_empty() => info.title = m.clean_text(),
            "creator" => {
                let role = m
                    .attr("role")
                    .map(str::to_string)
                    .or_else(|| refines(m.attr("id"), "role"));
                let name = m.clean_text();
                if name.is_empty() {
                    continue;
                }
                let file_as = m
                    .attr("file-as")
                    .map(str::to_string)
                    .or_else(|| refines(m.attr("id"), "file-as"));
                let p = person_from_name(&name, file_as.as_deref());
                match role.as_deref() {
                    None | Some("aut") => info.authors.push(p),
                    Some("trl") => info.translators.push(p),
                    _ => {}
                }
            }
            "language" if info.lang.is_empty() => info.lang = m.clean_text().to_lowercase(),
            "subject" => {
                let s = m.clean_text();
                if !s.is_empty() && !info.genres.contains(&s) {
                    info.genres.push(s);
                }
            }
            "description" if info.annotation.is_none() => {
                let raw = m.text();
                info.annotation = sanitize_html(&raw);
            }
            "publisher" if info.publisher.is_empty() => info.publisher = m.clean_text(),
            "date" if info.date.is_empty() => {
                info.date = m.clean_text();
                info.year = info.date.chars().take(4).collect();
            }
            "identifier" => {
                let v = m.clean_text();
                let scheme = m.attr("scheme").unwrap_or("").to_ascii_lowercase();
                if scheme == "isbn" || v.to_ascii_lowercase().starts_with("urn:isbn:") {
                    info.isbn = v.trim_start_matches("urn:isbn:").to_string();
                }
                if info.id.is_empty()
                    || package
                        .attr("unique-identifier")
                        .is_some_and(|u| m.attr("id") == Some(u))
                {
                    info.id = v;
                }
            }
            "meta" => {
                let name = m.attr("name").unwrap_or("");
                let content = m.attr("content").unwrap_or("");
                match name {
                    "calibre:series" if !content.trim().is_empty() => {
                        info.series = Some(collapse_ws(content))
                    }
                    "calibre:series_index" => info.serno = parse_serno(content),
                    _ => {}
                }
                if m.attr("property") == Some("belongs-to-collection") && info.series.is_none() {
                    let id = m.attr("id");
                    let ctype = refines(id, "collection-type");
                    if ctype.as_deref().is_none_or(|t| t == "series") {
                        info.series = Some(m.clean_text()).filter(|s| !s.is_empty());
                        info.serno = refines(id, "group-position").and_then(|p| parse_serno(&p));
                    }
                }
            }
            _ => {}
        }
    }

    // cover
    let manifest: Vec<&Element> = package
        .child("manifest")
        .map(|m| m.children_named("item").collect())
        .unwrap_or_default();
    let is_img = |i: &&Element| {
        i.attr("media-type")
            .is_some_and(|t| t.starts_with("image/"))
    };
    let cover_item = manifest
        .iter()
        .find(|i| {
            i.attr("properties")
                .is_some_and(|p| p.split_whitespace().any(|p| p == "cover-image"))
        })
        .or_else(|| {
            let id = md
                .iter()
                .find(|m| m.name == "meta" && m.attr("name") == Some("cover"))
                .and_then(|m| m.attr("content"))?;
            manifest
                .iter()
                .find(|i| i.attr("id") == Some(id) && is_img(i))
        })
        .or_else(|| {
            manifest.iter().find(|i| {
                is_img(i)
                    && (i
                        .attr("id")
                        .is_some_and(|s| s.to_ascii_lowercase().contains("cover"))
                        || i.attr("href")
                            .is_some_and(|s| s.to_ascii_lowercase().contains("cover")))
            })
        });
    if let Some(item) = cover_item
        && let Some(href) = item.attr("href")
    {
        let path = resolve_path(&opf_path, href);
        if let Some(data) = read_entry(&mut zip, &path) {
            let kind = images::sniff(&data);
            if matches!(
                kind,
                Kind::Jpeg | Kind::Png | Kind::Gif | Kind::Webp | Kind::Bmp
            ) && images::dimensions(&data, kind).is_some()
            {
                info.cover = Some(CoverImage {
                    data,
                    mime: kind.mime().to_string(),
                });
            }
        }
    }
    Ok(info)
}
