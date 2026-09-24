//! Typographic cover generation and cover labels (port of `InsertSeriaNumberToCover`).

use std::io::Cursor;
use std::sync::OnceLock;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use image::{DynamicImage, Rgb, RgbImage};

use crate::assets::Assets;

const INK: [u8; 3] = [0x26, 0x26, 0x2b];

fn background(assets: &Assets) -> &'static RgbImage {
    static BG: OnceLock<RgbImage> = OnceLock::new();
    BG.get_or_init(|| match image::load_from_memory(assets.cover_background()) {
        Ok(img) => img.to_rgb8(),
        Err(_) => RgbImage::from_pixel(800, 1280, Rgb([0xf2, 0xf2, 0xf2])),
    })
}

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

/// Word-wraps `text` (explicit `\n` respected) to `width` at `scale`.
fn wrap_lines(font: &FontRef, scale: PxScale, text: &str, width: f32) -> Option<Vec<(String, f32)>> {
    let sf = font.as_scaled(scale);
    let measure = |s: &str| -> f32 {
        let mut w = 0.0;
        let mut prev = None;
        for c in s.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                w += sf.kern(p, id);
            }
            w += sf.h_advance(id);
            prev = Some(id);
        }
        w
    };
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if measure(word) > width {
                return None;
            }
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if measure(&candidate) <= width {
                line = candidate;
            } else {
                let w = measure(&line);
                lines.push((std::mem::take(&mut line), w));
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            let w = measure(&line);
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
    for i in 0..3 {
        p.0[i] = (p.0[i] as f32 * (1.0 - a) + color[i] as f32 * a).round() as u8;
    }
}

struct Layout<'f> {
    font: FontRef<'f>,
    scale: PxScale,
    lines: Vec<(String, f32, f32)>, // text, x, baseline
    bbox: (i32, i32, i32, i32),
}

/// Lays out text in `rect`, shrinking the font until it fits.
fn layout_text<'f>(font_data: &'f [u8], text: &str, rect: &Rect, size: f32, valign: VAlign, right: bool) -> Option<Layout<'f>> {
    let font = FontRef::try_from_slice(font_data).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut px = size;
    let (lines, scale) = loop {
        let scale = PxScale::from(px);
        let lh = font.as_scaled(scale).height() + font.as_scaled(scale).line_gap();
        if let Some(lines) = wrap_lines(&font, scale, text, rect.w as f32)
            && (lines.len() as f32 * lh) <= rect.h as f32
        {
            break (lines, scale);
        }
        px *= 0.92;
        if px < 8.0 {
            return None;
        }
    };
    let sf = font.as_scaled(scale);
    let lh = sf.height() + sf.line_gap();
    let total = lines.len() as f32 * lh;
    let top = match valign {
        VAlign::Top => rect.y as f32,
        VAlign::Center => rect.y as f32 + (rect.h as f32 - total) / 2.0,
        VAlign::Bottom => rect.y as f32 + rect.h as f32 - total,
    };
    let xpos = |w: f32| if right { rect.x as f32 + rect.w as f32 - w } else { rect.x as f32 + (rect.w as f32 - w) / 2.0 };
    let maxw = lines.iter().map(|l| l.1).fold(0.0f32, f32::max);
    let bx0 = xpos(maxw);
    let bbox = (bx0 as i32, top as i32, (bx0 + maxw).ceil() as i32, (top + total).ceil() as i32);
    let lines = lines.into_iter().enumerate().map(|(i, (t, w))| (t, xpos(w), top + i as f32 * lh + sf.ascent())).collect();
    Some(Layout { font, scale, lines, bbox })
}

fn paint(img: &mut RgbImage, l: &Layout, color: [u8; 3]) {
    let sf = l.font.as_scaled(l.scale);
    for (line, x0, baseline) in &l.lines {
        let mut x = *x0;
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let g = id.with_scale_and_position(l.scale, point(x, *baseline));
            x += sf.h_advance(id);
            prev = Some(id);
            if let Some(og) = l.font.outline_glyph(g) {
                let b = og.px_bounds();
                og.draw(|gx, gy, cov| blend(img, b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, color, cov));
            }
        }
    }
}

fn draw_text(img: &mut RgbImage, font_data: &[u8], text: &str, rect: &Rect, size: f32, valign: VAlign) {
    if let Some(l) = layout_text(font_data, text, rect, size, valign, false) {
        paint(img, &l, INK);
    }
}

pub(crate) fn encode_jpeg(img: &RgbImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut out), 80);
    img.write_with_encoder(enc).ok()?;
    Some(out)
}

/// Draws a cover: author at the top, title in the middle, `bottom` (series / label) below.
pub fn generate_cover(assets: &Assets, author: &str, title: &str, bottom: &str) -> Vec<u8> {
    let mut img = background(assets).clone();
    let (w, h) = (img.width() as i32, img.height() as i32);
    let delta = h / 50;
    let delta2 = h / 100;
    let rh = h - delta * 2;
    let quarter = rh / 4;
    let regular = assets.cover_font(false);
    let bold = assets.cover_font(true);
    // thin frame
    for t in 0..3 {
        let m = delta / 2 + t;
        for x in m..w - m {
            blend(&mut img, x, m, INK, 0.6);
            blend(&mut img, x, h - 1 - m, INK, 0.6);
        }
        for y in m..h - m {
            blend(&mut img, m, y, INK, 0.6);
            blend(&mut img, w - 1 - m, y, INK, 0.6);
        }
    }
    let inner = delta * 2;
    draw_text(&mut img, regular, author, &Rect { x: inner, y: inner, w: w - inner * 2, h: quarter - delta2 }, h as f32 / 15.0, VAlign::Top);
    draw_text(
        &mut img,
        bold,
        title,
        &Rect { x: inner, y: delta + quarter + delta2, w: w - inner * 2, h: rh - quarter * 2 - delta2 * 2 },
        h as f32 / 12.0,
        VAlign::Center,
    );
    draw_text(&mut img, regular, bottom, &Rect { x: inner, y: delta + rh - quarter + delta2, w: w - inner * 2, h: quarter - delta2 - delta }, h as f32 / 17.0, VAlign::Bottom);
    encode_jpeg(&img).unwrap_or_default()
}

/// Draws `label` in the top-right corner of an existing cover on a translucent white box.
/// Returns `None` if the image cannot be decoded.
pub fn label_cover(assets: &Assets, cover: &[u8], label: &str) -> Option<Vec<u8>> {
    let img: DynamicImage = image::load_from_memory(cover).ok()?;
    let mut img = img.to_rgb8();
    let (w, h) = (img.width() as i32, img.height() as i32);
    let pad = (h / 60).max(2);
    let rect = Rect { x: w / 3, y: pad, w: w - w / 3 - pad, h: h / 6 };
    let l = layout_text(assets.cover_font(true), label, &rect, h as f32 / 15.0, VAlign::Top, true)?;
    let (x0, y0, x1, y1) = l.bbox;
    for y in (y0 - pad / 2).max(0)..(y1 + pad / 2).min(h) {
        for x in (x0 - pad).max(0)..(x1 + pad).min(w) {
            blend(&mut img, x, y, [255, 255, 255], 168.0 / 255.0);
        }
    }
    paint(&mut img, &l, INK);
    encode_jpeg(&img)
}
