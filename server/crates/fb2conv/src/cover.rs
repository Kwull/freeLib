//! Covers: typographic cover generation, validation and resizing of book covers, and cover
//! labels (port of `InsertSeriaNumberToCover`).
//!
//! Generated covers are 1600×2560 JPEGs (the 1:1.6 shape Kindle, Apple Books and Kobo grids
//! use) on a flat colour with PT Serif (full Cyrillic): author at the top, the title in the
//! middle, series and number at the bottom. Flat colour compresses well: typically 60–120 KB.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use image::{DynamicImage, Rgb, RgbImage};

use crate::assets::Assets;
use crate::images::{Kind, Prepared};

pub const COVER_WIDTH: u32 = 1600;
pub const COVER_HEIGHT: u32 = 2560;

const INK: [u8; 3] = [0x26, 0x26, 0x2b];

/// Background, text and accent colours. Every background is dark enough that light text stays
/// readable in greyscale on e-ink.
const PALETTES: &[([u8; 3], [u8; 3], [u8; 3])] = &[
    ([0x1f, 0x4e, 0x4a], [0xf4, 0xee, 0xe1], [0xd2, 0xb4, 0x7a]), // teal
    ([0x5e, 0x1f, 0x2b], [0xf6, 0xec, 0xe0], [0xe0, 0xb8, 0x74]), // burgundy
    ([0x1e, 0x2a, 0x44], [0xf2, 0xee, 0xe4], [0xcf, 0xad, 0x72]), // navy
    ([0x2c, 0x46, 0x32], [0xf3, 0xef, 0xe0], [0xd8, 0xc0, 0x8a]), // forest
    ([0x3e, 0x2a, 0x47], [0xf4, 0xee, 0xe6], [0xd6, 0xb2, 0x80]), // plum
    ([0x2b, 0x2b, 0x2e], [0xf1, 0xed, 0xe4], [0xc8, 0xa0, 0x5a]), // charcoal
    ([0x6b, 0x33, 0x1c], [0xf8, 0xef, 0xe2], [0xea, 0xcf, 0x9b]), // rust
    ([0x2f, 0x44, 0x58], [0xf1, 0xee, 0xe6], [0xdc, 0xc0, 0x8c]), // slate
];

#[derive(Clone, Copy)]
enum VAlign {
    Top,
    Center,
    Bottom,
}

struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn measure(font: &FontRef, scale: PxScale, s: &str) -> f32 {
    let sf = font.as_scaled(scale);
    let mut w = 0.0;
    let mut prev = None;
    for c in s.chars().map(unglue) {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Word-wraps `text` (explicit `\n` respected) to `width` at `scale`.
fn wrap_lines(
    font: &FontRef,
    scale: PxScale,
    text: &str,
    width: f32,
) -> Option<Vec<(String, f32)>> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ').filter(|w| !w.trim().is_empty()) {
            if measure(font, scale, word) > width {
                return None;
            }
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if measure(font, scale, &candidate) <= width {
                line = candidate;
            } else {
                let w = measure(font, scale, &line);
                lines.push((std::mem::take(&mut line), w));
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            let w = measure(font, scale, &line);
            lines.push((line, w));
        }
    }
    Some(lines)
}

fn blend(img: &mut RgbImage, x: i32, y: i32, color: [u8; 3], a: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 || a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let p = img.get_pixel_mut(x as u32, y as u32);
    for (ch, &col) in p.0.iter_mut().zip(color.iter()) {
        *ch = (*ch as f32 * (1.0 - a) + col as f32 * a).round() as u8;
    }
}

fn fill_rect(img: &mut RgbImage, x0: i32, y0: i32, x1: i32, y1: i32, color: [u8; 3]) {
    for y in y0.max(0)..y1.min(img.height() as i32) {
        for x in x0.max(0)..x1.min(img.width() as i32) {
            img.put_pixel(x as u32, y as u32, Rgb(color));
        }
    }
}

struct Layout<'f> {
    font: FontRef<'f>,
    scale: PxScale,
    lines: Vec<(String, f32, f32)>, // text, x, baseline
    bbox: (i32, i32, i32, i32),
}

/// Lays out text in `rect`, shrinking the font until it fits. With `balance`, the wrap width
/// is then narrowed as long as the line count stays the same, so lines get similar lengths
/// instead of one long line and a lone word.
fn layout_text<'f>(
    font_data: &'f [u8],
    text: &str,
    rect: &Rect,
    size: f32,
    valign: VAlign,
    right: bool,
    balance: bool,
) -> Option<Layout<'f>> {
    let font = FontRef::try_from_slice(font_data).ok()?;
    let text = glue_short_words(text.trim());
    let text = text.as_str();
    if text.is_empty() {
        return None;
    }
    let mut px = size;
    let (mut lines, scale) = loop {
        let scale = PxScale::from(px);
        let lh = font.as_scaled(scale).height() * 1.08;
        if let Some(lines) = wrap_lines(&font, scale, text, rect.w as f32)
            && (lines.len() as f32 * lh) <= rect.h as f32
        {
            break (lines, scale);
        }
        px *= 0.93;
        if px < 8.0 {
            return None;
        }
    };
    if balance && lines.len() > 1 {
        let n = lines.len();
        let mut w = rect.w as f32;
        while w > rect.w as f32 * 0.5 {
            w *= 0.96;
            match wrap_lines(&font, scale, text, w) {
                Some(l) if l.len() == n => lines = l,
                _ => break,
            }
        }
    }
    let sf = font.as_scaled(scale);
    let lh = sf.height() * 1.08;
    let total = lines.len() as f32 * lh;
    let top = match valign {
        VAlign::Top => rect.y as f32,
        VAlign::Center => rect.y as f32 + (rect.h as f32 - total) / 2.0,
        VAlign::Bottom => rect.y as f32 + rect.h as f32 - total,
    };
    let xpos = |w: f32| {
        if right {
            rect.x as f32 + rect.w as f32 - w
        } else {
            rect.x as f32 + (rect.w as f32 - w) / 2.0
        }
    };
    let maxw = lines.iter().map(|l| l.1).fold(0.0f32, f32::max);
    let bx0 = xpos(maxw);
    let bbox = (
        bx0 as i32,
        top as i32,
        (bx0 + maxw).ceil() as i32,
        (top + total).ceil() as i32,
    );
    let lines = lines
        .into_iter()
        .enumerate()
        .map(|(i, (t, w))| (t, xpos(w), top + i as f32 * lh + sf.ascent()))
        .collect();
    Some(Layout {
        font,
        scale,
        lines,
        bbox,
    })
}

fn paint(img: &mut RgbImage, l: &Layout, color: [u8; 3]) {
    let sf = l.font.as_scaled(l.scale);
    for (line, x0, baseline) in &l.lines {
        let mut x = *x0;
        let mut prev = None;
        for c in line.chars().map(unglue) {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(l.scale, point(x, *baseline));
            x += sf.h_advance(id);
            prev = Some(id);
            if let Some(og) = l.font.outline_glyph(g) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| {
                    blend(
                        img,
                        b.min.x as i32 + gx as i32,
                        b.min.y as i32 + gy as i32,
                        color,
                        cov,
                    )
                });
            }
        }
    }
}

/// Baseline JPEG, 4:2:0 chroma, optimised Huffman tables (flat colour costs almost nothing).
pub(crate) fn encode_jpeg(img: &RgbImage, quality: u8) -> Option<Vec<u8>> {
    let (w, h) = (
        u16::try_from(img.width()).ok()?,
        u16::try_from(img.height()).ok()?,
    );
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, quality);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
    enc.set_optimized_huffman_tables(true);
    enc.encode(img.as_raw(), w, h, jpeg_encoder::ColorType::Rgb)
        .ok()?;
    Some(out)
}

/// Glues one- and two-letter words (prepositions and conjunctions: "в", "и", "на", "of") to
/// the following word with a no-break space, as Russian typesetting does, so they never end
/// a line.
fn glue_short_words(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, para) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let words: Vec<&str> = para.split_whitespace().collect();
        for (j, w) in words.iter().enumerate() {
            out.push_str(w);
            if j + 1 < words.len() {
                let short = w.chars().count() <= 2 && w.chars().all(char::is_alphabetic);
                out.push(if short { NBSP } else { ' ' });
            }
        }
    }
    out
}

const NBSP: char = '\u{a0}';

fn unglue(c: char) -> char {
    if c == NBSP { ' ' } else { c }
}

fn palette(author: &str, title: &str) -> ([u8; 3], [u8; 3], [u8; 3]) {
    let h = crate::epub::hash128(format!("{author}\0{title}").as_bytes());
    PALETTES[(h % PALETTES.len() as u128) as usize]
}

/// A small diamond (ornament) centred at `(cx, cy)`.
fn diamond(img: &mut RgbImage, cx: i32, cy: i32, r: i32, color: [u8; 3]) {
    for y in -r..=r {
        for x in -r..=r {
            let d = (x.abs() + y.abs()) as f32;
            blend(
                img,
                cx + x,
                cy + y,
                color,
                (r as f32 + 0.5 - d).clamp(0.0, 1.0),
            );
        }
    }
}

/// Draws a cover: author at the top, the title in the middle, `bottom` below — its first line
/// (the series) in italics, further lines (the number, "Книга 3") upright.
pub fn generate_cover(assets: &Assets, author: &str, title: &str, bottom: &str) -> Vec<u8> {
    let (w, h) = (COVER_WIDTH as i32, COVER_HEIGHT as i32);
    let (bg, fg, accent) = palette(author, title);
    let mut img = RgbImage::from_pixel(w as u32, h as u32, Rgb(bg));
    let regular = assets.cover_font(false);
    let bold = assets.cover_font(true);
    let italic = assets.cover_italic();

    // double frame: a thin accent line inside a hairline
    let m = 64;
    for (inset, t, a) in [(m, 5, 1.0f32), (m + 22, 2, 0.55)] {
        for k in 0..t {
            let i = inset + k;
            for x in i..w - i {
                blend(&mut img, x, i, accent, a);
                blend(&mut img, x, h - 1 - i, accent, a);
            }
            for y in i..h - i {
                blend(&mut img, i, y, accent, a);
                blend(&mut img, w - 1 - i, y, accent, a);
            }
        }
    }
    let side = 190;
    let text_w = w - side * 2;

    // author
    if let Some(l) = layout_text(
        regular,
        author,
        &Rect {
            x: side,
            y: 250,
            w: text_w,
            h: 420,
        },
        104.0,
        VAlign::Bottom,
        false,
        true,
    ) {
        paint(&mut img, &l, fg);
    }
    // ornament
    let cy = 770;
    fill_rect(&mut img, w / 2 - 220, cy - 2, w / 2 - 40, cy + 2, accent);
    fill_rect(&mut img, w / 2 + 40, cy - 2, w / 2 + 220, cy + 2, accent);
    diamond(&mut img, w / 2, cy, 16, accent);

    // title
    if let Some(l) = layout_text(
        bold,
        title,
        &Rect {
            x: side - 30,
            y: 880,
            w: text_w + 60,
            h: 1060,
        },
        196.0,
        VAlign::Center,
        false,
        true,
    ) {
        paint(&mut img, &l, fg);
    }

    // series / label
    let mut lines = bottom.trim().splitn(2, '\n');
    let series = lines.next().unwrap_or("").trim();
    let rest = lines.next().unwrap_or("").trim();
    if !series.is_empty() || !rest.is_empty() {
        let ry = 2040;
        fill_rect(&mut img, w / 2 - 120, ry, w / 2 + 120, ry + 4, accent);
        let series_rect = Rect {
            x: side,
            y: ry + 50,
            w: text_w,
            h: if rest.is_empty() { 300 } else { 190 },
        };
        let mut next_y = ry + 50;
        if let Some(l) = layout_text(italic, series, &series_rect, 84.0, VAlign::Top, false, true) {
            paint(&mut img, &l, accent);
            next_y = l.bbox.3 + 30;
        }
        if let Some(l) = layout_text(
            regular,
            rest,
            &Rect {
                x: side,
                y: next_y,
                w: text_w,
                h: 110,
            },
            70.0,
            VAlign::Top,
            false,
            false,
        ) {
            paint(&mut img, &l, fg);
        }
    }
    encode_jpeg(&img, 80).unwrap_or_default()
}

/// Whether a cover image has a usable size: covers smaller than 100 px or thinner than 1:4
/// are placeholders or broken, and are replaced by a generated one.
pub(crate) fn usable(width: u32, height: u32) -> bool {
    let (a, b) = (width.min(height), width.max(height));
    a >= 100 && b <= a * 4
}

/// Whether an image has its end marker (JPEG EOI, PNG IEND): FB2 covers are often cut off by
/// a truncated download, and decoders happily return a half-grey picture for those.
fn complete(data: &[u8], kind: Kind) -> bool {
    let tail = &data[data.len().saturating_sub(64 * 1024)..];
    match kind {
        Kind::Jpeg => tail.windows(2).any(|w| w == [0xff, 0xd9]),
        Kind::Png => tail.windows(4).any(|w| w == b"IEND"),
        _ => true,
    }
}

/// Largest cover kept as it is (bytes); bigger ones are re-encoded.
const MAX_COVER_BYTES: usize = 1536 * 1024;

/// Validates a book's own cover: decodes it fully (a broken or truncated image is `None`),
/// rejects unusable sizes, and scales covers much larger than 1600×2560 (or heavier than
/// 1.5 MB) down to a JPEG that fits 1600×2560. SVG covers are kept as they are.
pub(crate) fn checked(c: Prepared) -> Option<Prepared> {
    if c.kind == Kind::Svg {
        return Some(c);
    }
    if !usable(c.width, c.height) {
        return None;
    }
    if !complete(&c.data, c.kind) {
        return None;
    }
    let img = crate::limit::decode_image(&c.data)?;
    let too_big = c.width > COVER_WIDTH * 5 / 4 || c.height > COVER_HEIGHT * 5 / 4;
    if !too_big && c.data.len() <= MAX_COVER_BYTES {
        return Some(c);
    }
    let img = if too_big {
        img.resize(
            COVER_WIDTH,
            COVER_HEIGHT,
            image::imageops::FilterType::CatmullRom,
        )
    } else {
        img
    };
    // flatten transparency onto white
    let rgba = img.to_rgba8();
    let mut rgb = RgbImage::new(rgba.width(), rgba.height());
    for (d, s) in rgb.pixels_mut().zip(rgba.pixels()) {
        let a = s.0[3] as f32 / 255.0;
        for i in 0..3 {
            d.0[i] = (s.0[i] as f32 * a + 255.0 * (1.0 - a)).round() as u8;
        }
    }
    let data = encode_jpeg(&rgb, 85)?;
    Some(Prepared {
        width: rgb.width(),
        height: rgb.height(),
        data,
        kind: Kind::Jpeg,
    })
}

/// Draws `label` in the top-right corner of an existing cover on a translucent white box.
/// Returns `None` if the image cannot be decoded.
pub fn label_cover(assets: &Assets, cover: &[u8], label: &str) -> Option<Vec<u8>> {
    let img: DynamicImage = crate::limit::decode_image(cover)?;
    let mut img = img.to_rgb8();
    let (w, h) = (img.width() as i32, img.height() as i32);
    let pad = (h / 60).max(2);
    let rect = Rect {
        x: w / 3,
        y: pad,
        w: w - w / 3 - pad,
        h: h / 6,
    };
    let l = layout_text(
        assets.cover_font(true),
        label,
        &rect,
        h as f32 / 15.0,
        VAlign::Top,
        true,
        false,
    )?;
    let (x0, y0, x1, y1) = l.bbox;
    for y in (y0 - pad / 2).max(0)..(y1 + pad / 2).min(h) {
        for x in (x0 - pad).max(0)..(x1 + pad).min(w) {
            blend(&mut img, x, y, [255, 255, 255], 168.0 / 255.0);
        }
    }
    paint(&mut img, &l, INK);
    encode_jpeg(&img, 85)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn generated_cover_is_light_and_device_sized() {
        let a = Assets::shared();
        let t = std::time::Instant::now();
        let data = generate_cover(
            a,
            "Аркадий Стругацкий, Борис Стругацкий",
            "Понедельник начинается в субботу",
            "НИИЧАВО\nКнига 1",
        );
        let img = image::load_from_memory(&data).unwrap();
        assert_eq!((img.width(), img.height()), (COVER_WIDTH, COVER_HEIGHT));
        assert!(data.len() < 180 * 1024, "{} bytes", data.len());
        eprintln!("cover: {} bytes in {:?}", data.len(), t.elapsed());
        if let Ok(p) = std::env::var("FREELIB_COVER_SAMPLE") {
            std::fs::write(p, &data).unwrap();
        }
        // no text at all still gives a valid cover
        let empty = generate_cover(a, "", "", "");
        assert!(image::load_from_memory(&empty).is_ok());
    }

    #[test]
    fn cover_checks() {
        assert!(usable(300, 450));
        assert!(!usable(60, 90));
        assert!(!usable(100, 900));
        let mut big = Vec::new();
        RgbImage::from_pixel(3000, 4800, Rgb([200, 10, 10]))
            .write_to(&mut Cursor::new(&mut big), image::ImageFormat::Png)
            .unwrap();
        let p = checked(Prepared {
            data: big,
            kind: Kind::Png,
            width: 3000,
            height: 4800,
        })
        .unwrap();
        assert_eq!(p.kind, Kind::Jpeg);
        assert_eq!((p.width, p.height), (1600, 2560));
        // truncated JPEG → broken
        let mut j = Vec::new();
        RgbImage::from_pixel(400, 600, Rgb([1, 2, 3]))
            .write_to(&mut Cursor::new(&mut j), image::ImageFormat::Jpeg)
            .unwrap();
        j.truncate(j.len() / 3);
        assert!(
            checked(Prepared {
                data: j,
                kind: Kind::Jpeg,
                width: 400,
                height: 600
            })
            .is_none()
        );
    }
}
