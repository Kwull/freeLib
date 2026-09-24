//! EPUB → Kobo KEPUB (what kepubify does): every sentence of text is wrapped in
//! `<span class="koboSpan" id="kobo.P.S">`, images too, and the body content is wrapped in
//! `<div id="book-columns"><div id="book-inner">`. Kobo firmware uses these spans for
//! reading position, highlights and page statistics.

use std::io::{Cursor, Read, Write};

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::epub::{deflated, stored, zip_err};
use crate::{Error, Result};

const KOBO_STYLE: &str = "<style type=\"text/css\">div#book-inner { margin-top: 0; margin-bottom: 0; }</style>";

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "div" | "li" | "td" | "th" | "blockquote" | "dt" | "dd" | "figcaption" | "pre" | "aside" | "section" | "tr" | "caption" | "nav"
    )
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

struct State {
    para: u32,
    seg: u32,
    skip_depth: u32,
    in_body: bool,
    pending: String,
}

impl State {
    fn next_para(&mut self) {
        self.para += 1;
        self.seg = 0;
    }

    fn span_open(&mut self, out: &mut String) {
        if self.para == 0 {
            self.para = 1;
        }
        self.seg += 1;
        out.push_str(&format!("<span class=\"koboSpan\" id=\"kobo.{}.{}\">", self.para, self.seg));
    }

    /// Emits buffered raw (escaped) text split into sentences.
    fn flush(&mut self, out: &mut String) {
        if self.pending.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending);
        if self.skip_depth > 0 || !self.in_body || text.trim().is_empty() {
            out.push_str(&text);
            return;
        }
        // leading whitespace stays outside the spans
        let trimmed = text.trim_start();
        out.push_str(&text[..text.len() - trimmed.len()]);
        for sentence in split_sentences(trimmed) {
            let core = sentence.trim_end();
            if core.is_empty() {
                out.push_str(sentence);
                continue;
            }
            self.span_open(out);
            out.push_str(sentence);
            out.push_str("</span>");
        }
    }
}

/// Splits after `.`, `!`, `?`, `…` (plus closing quotes/brackets) followed by whitespace; the
/// whitespace stays with the preceding sentence.
fn split_sentences(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        if matches!(c, '.' | '!' | '?' | '…') {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j].1, '.' | '!' | '?' | '…' | '»' | '"' | '\'' | ')' | ']' | '”' | '’') {
                j += 1;
            }
            if j < chars.len() && chars[j].1.is_whitespace() {
                while j < chars.len() && chars[j].1.is_whitespace() {
                    j += 1;
                }
                if j < chars.len() {
                    let end = chars[j].0;
                    parts.push(&s[start..end]);
                    start = end;
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

/// Adds Kobo spans to one XHTML document. Returns `None` if it is already a KEPUB document.
pub fn kepubify_xhtml(src: &str) -> Option<String> {
    if src.contains("koboSpan") {
        return None;
    }
    let mut reader = Reader::from_str(src);
    {
        let c = reader.config_mut();
        c.check_end_names = false;
        c.allow_unmatched_ends = true;
        c.allow_dangling_amp = true;
    }
    let mut out = String::with_capacity(src.len() + src.len() / 3);
    let mut st = State { para: 0, seg: 0, skip_depth: 0, in_body: false, pending: String::new() };
    loop {
        let start = reader.buffer_position() as usize;
        let ev = match reader.read_event() {
            Ok(ev) => ev,
            Err(_) => return None,
        };
        let end = reader.buffer_position() as usize;
        let raw = &src[start.min(src.len())..end.min(src.len())];
        match ev {
            Event::Text(_) | Event::GeneralRef(_) => {
                st.pending.push_str(raw);
                continue;
            }
            Event::Eof => {
                st.flush(&mut out);
                break;
            }
            _ => st.flush(&mut out),
        }
        match ev {
            Event::Start(e) => {
                let name = local(e.name().as_ref()).to_ascii_lowercase();
                match name.as_str() {
                    "body" => {
                        out.push_str(raw);
                        out.push_str("<div id=\"book-columns\"><div id=\"book-inner\">");
                        st.in_body = true;
                        continue;
                    }
                    "script" | "style" | "svg" | "math" | "pre" | "head" | "title" => st.skip_depth += 1,
                    n if is_block(n) => st.next_para(),
                    _ => {}
                }
                out.push_str(raw);
            }
            Event::End(e) => {
                let name = local(e.name().as_ref()).to_ascii_lowercase();
                match name.as_str() {
                    "body" => {
                        out.push_str("</div></div>");
                        st.in_body = false;
                    }
                    "head" => {
                        out.push_str(KOBO_STYLE);
                        st.skip_depth = st.skip_depth.saturating_sub(1);
                    }
                    "script" | "style" | "svg" | "math" | "pre" | "title" => st.skip_depth = st.skip_depth.saturating_sub(1),
                    _ => {}
                }
                out.push_str(raw);
            }
            Event::Empty(e) => {
                let name = local(e.name().as_ref()).to_ascii_lowercase();
                if name == "img" && st.in_body && st.skip_depth == 0 {
                    st.next_para();
                    st.span_open(&mut out);
                    out.push_str(raw);
                    out.push_str("</span>");
                } else {
                    out.push_str(raw);
                }
            }
            _ => out.push_str(raw),
        }
    }
    Some(out)
}

/// Converts an EPUB into a Kobo KEPUB (rewrites every XHTML content document; other entries are
/// copied unchanged). The result should be saved as `*.kepub.epub`.
pub fn to_kepub(epub: &[u8]) -> Result<Vec<u8>> {
    let mut zin = zip::ZipArchive::new(Cursor::new(epub)).map_err(zip_err)?;
    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::with_capacity(epub.len() + epub.len() / 4)));
    zw.start_file("mimetype", stored()).map_err(zip_err)?;
    zw.write_all(b"application/epub+zip")?;
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).map_err(zip_err)?;
        let name = f.name().to_string();
        if name == "mimetype" || f.is_dir() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".xhtml") || lower.ends_with(".html") || lower.ends_with(".htm") {
            let mut data = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut data)?;
            drop(f);
            let text = String::from_utf8_lossy(&data);
            let is_nav = text.contains("epub:type=\"toc\"") && text.contains("<nav");
            let converted = if is_nav { None } else { kepubify_xhtml(&text) };
            zw.start_file(name, deflated()).map_err(zip_err)?;
            match converted {
                Some(c) => zw.write_all(c.as_bytes())?,
                None => zw.write_all(&data)?,
            }
        } else {
            zw.raw_copy_file(f).map_err(zip_err)?;
        }
    }
    let cur = zw.finish().map_err(|e| Error::Zip(e.to_string()))?;
    Ok(cur.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences() {
        assert_eq!(split_sentences("Раз. Два! «Три?» Четыре"), vec!["Раз. ", "Два! ", "«Три?» ", "Четыре"]);
        assert_eq!(split_sentences("т.е. всё"), vec!["т.е. ", "всё"]);
    }

    #[test]
    fn spans() {
        let src = "<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>T. X</title></head><body><h2>Глава</h2><p>Раз. Два &amp; три.</p><p><img src=\"a.jpg\" alt=\"\"/></p></body></html>";
        let out = kepubify_xhtml(src).unwrap();
        assert!(out.contains("<title>T. X</title>"));
        assert!(out.contains("<body><div id=\"book-columns\"><div id=\"book-inner\"><h2><span class=\"koboSpan\" id=\"kobo.1.1\">Глава</span></h2>"), "{out}");
        assert!(out.contains("<p><span class=\"koboSpan\" id=\"kobo.2.1\">Раз. </span><span class=\"koboSpan\" id=\"kobo.2.2\">Два &amp; три.</span></p>"), "{out}");
        assert!(out.contains("<span class=\"koboSpan\" id=\"kobo.4.1\"><img src=\"a.jpg\" alt=\"\"/></span>"), "{out}");
        assert!(out.contains("</div></div></body>"));
        assert!(kepubify_xhtml(&out).is_none());
    }
}
