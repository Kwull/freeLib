//! Generated cover placeholder: an SVG tile that looks like the SPA's own placeholder
//! (`web/src/lib/components/CoverThumb.svelte`), used by `GET .../cover?size=thumb` when the
//! book has no embedded cover. Deterministic from the title (and first author), so it needs no
//! cache invalidation beyond its own ETag.

use freelib_catalog::AuthorRef;

/// Same 7-colour palette and hash as `initialsBg()` in `web/src/lib/utils/format.ts`, so a
/// placeholder tile looks identical whether the SPA or the server drew it.
const PALETTE: [&str; 7] = [
    "#2F4A5A", "#6B3A2E", "#3E5A3A", "#4A4063", "#5A4A2F", "#3A4A5C", "#5A3A4A",
];

/// Mirrors the JS `initialsBg`: `h = (h*31 + charCode) >>> 0` over UTF-16 code units.
pub fn bg_color(seed: &str) -> &'static str {
    let mut h: u32 = 0;
    for c in seed.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as u32);
    }
    PALETTE[(h as usize) % PALETTE.len()]
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Greedy word-wrap to `max_chars` per line, at most `max_lines` lines; the last line is
/// ellipsised when text remains.
fn wrap(text: &str, max_chars: usize, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let candidate = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if candidate.chars().count() > max_chars && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
            if lines.len() == max_lines {
                break;
            }
            cur = word.to_string();
        } else {
            cur = candidate;
        }
    }
    if lines.len() < max_lines && !cur.is_empty() {
        lines.push(cur);
    }
    if lines.len() == max_lines {
        // Signal truncation with an ellipsis if there is more text than fits.
        let consumed: usize = lines.iter().map(|l| l.chars().count() + 1).sum();
        if consumed < text.chars().count() {
            let last = lines.last_mut().unwrap();
            while last.chars().count() + 1 > max_chars && !last.is_empty() {
                last.pop();
            }
            last.push('…');
        }
    }
    lines
}

/// Builds the placeholder tile: `width` x `height` SVG, background colour derived from the
/// title, first-author line near the top (small caps, like the prototype), title at the
/// bottom, wrapped over a few lines.
pub fn svg(title: &str, authors: &[AuthorRef], width: u32, height: u32) -> Vec<u8> {
    let bg = bg_color(title);
    let title = if title.trim().is_empty() {
        "—"
    } else {
        title.trim()
    };
    let author_line = authors
        .first()
        .map(|a| a.name.to_uppercase())
        .unwrap_or_default();

    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\" aria-label=\"{}\">",
        xml_escape(title)
    ));
    s.push_str(&format!(
        "<rect width=\"{width}\" height=\"{height}\" rx=\"4\" fill=\"{bg}\"/>"
    ));
    let pad = (width as f32 * 0.09).max(6.0);
    if !author_line.is_empty() {
        let lines = wrap(&author_line, 20, 1);
        if let Some(line) = lines.first() {
            s.push_str(&format!(
                "<text x=\"{pad}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"{}\" letter-spacing=\"0.06em\" fill=\"#F2EEE6\">{}</text>",
                pad + 8.0,
                (width as f32 * 0.062).max(8.0),
                xml_escape(line)
            ));
        }
    }
    let title_lines = wrap(title, 14, 5);
    let font_size = (width as f32 * 0.095).max(10.0);
    let line_h = font_size * 1.25;
    let base_y = height as f32 - pad - (title_lines.len() as f32 - 1.0) * line_h;
    s.push_str(&format!(
        "<text x=\"{pad}\" y=\"{base_y}\" font-family=\"Georgia, 'Times New Roman', serif\" font-size=\"{font_size}\" fill=\"#FFFFFF\">"
    ));
    for (i, line) in title_lines.iter().enumerate() {
        let dy = if i == 0 { 0.0 } else { line_h };
        s.push_str(&format!(
            "<tspan x=\"{pad}\" dy=\"{dy}\">{}</tspan>",
            xml_escape(line)
        ));
    }
    s.push_str("</text></svg>");
    s.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_color() {
        assert_eq!(bg_color("Полдень, XXII век"), bg_color("Полдень, XXII век"));
    }

    #[test]
    fn wraps_and_escapes() {
        let bytes = svg(
            "A very long title that should wrap over several lines <script>",
            &[AuthorRef {
                id: 1,
                name: "Стругацкий Аркадий".into(),
            }],
            160,
            240,
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("<svg"));
        assert!(!text.contains("<script>"));
        assert!(text.contains("&lt;script&gt;"));
        assert!(text.contains("СТРУГАЦКИЙ"));
    }
}
