//! Image sniffing, validation and conversion to EPUB core media types.

use std::io::Cursor;

use image::{ImageFormat, ImageReader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Gif,
    Webp,
    Bmp,
    Svg,
    Unknown,
}

impl Kind {
    pub fn mime(self) -> &'static str {
        match self {
            Kind::Jpeg => "image/jpeg",
            Kind::Png => "image/png",
            Kind::Gif => "image/gif",
            Kind::Webp => "image/webp",
            Kind::Bmp => "image/bmp",
            Kind::Svg => "image/svg+xml",
            Kind::Unknown => "application/octet-stream",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Jpeg => "jpg",
            Kind::Png => "png",
            Kind::Gif => "gif",
            Kind::Webp => "webp",
            Kind::Bmp => "bmp",
            Kind::Svg => "svg",
            Kind::Unknown => "bin",
        }
    }
    fn format(self) -> Option<ImageFormat> {
        Some(match self {
            Kind::Jpeg => ImageFormat::Jpeg,
            Kind::Png => ImageFormat::Png,
            Kind::Gif => ImageFormat::Gif,
            Kind::Webp => ImageFormat::WebP,
            Kind::Bmp => ImageFormat::Bmp,
            _ => return None,
        })
    }
}

pub fn sniff(b: &[u8]) -> Kind {
    if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Kind::Jpeg
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Kind::Png
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Kind::Gif
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Kind::Webp
    } else if b.starts_with(b"BM") && b.len() > 26 {
        Kind::Bmp
    } else {
        let head = String::from_utf8_lossy(&b[..b.len().min(512)]);
        if head.contains("<svg") {
            Kind::Svg
        } else {
            Kind::Unknown
        }
    }
}

/// An image ready to be put into an EPUB: bytes in a core media type plus its dimensions.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub data: Vec<u8>,
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
}

pub fn dimensions(data: &[u8], kind: Kind) -> Option<(u32, u32)> {
    let fmt = kind.format()?;
    let r = ImageReader::with_format(Cursor::new(data), fmt);
    r.into_dimensions().ok().filter(|&(w, h)| w > 0 && h > 0)
}

/// Validates an image and converts it to JPEG/PNG/GIF when needed.
/// JPEG, PNG and GIF are only header-checked (cheap); WebP, BMP and anything the `image`
/// crate can guess are fully decoded and re-encoded. Returns `None` for broken images.
pub fn prepare(data: Vec<u8>) -> Option<Prepared> {
    let kind = sniff(&data);
    match kind {
        Kind::Jpeg | Kind::Png | Kind::Gif => {
            let (width, height) = dimensions(&data, kind)?;
            Some(Prepared {
                data,
                kind,
                width,
                height,
            })
        }
        Kind::Svg => Some(Prepared {
            data,
            kind,
            width: 600,
            height: 800,
        }),
        _ => {
            let img = crate::limit::decode_image(&data)?;
            let (width, height) = (img.width(), img.height());
            if width == 0 || height == 0 {
                return None;
            }
            let mut out = Vec::new();
            let kind = if img.color().has_alpha() {
                img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
                    .ok()?;
                Kind::Png
            } else {
                img.to_rgb8()
                    .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
                    .ok()?;
                Kind::Jpeg
            };
            Some(Prepared {
                data: out,
                kind,
                width,
                height,
            })
        }
    }
}
