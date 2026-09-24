//! Bounded reads and image decoding (zip bombs, huge declared sizes, decompression bombs).

use std::io::{self, Cursor, Read};

use image::{DynamicImage, ImageReader, Limits};

/// Largest initial buffer reserved from an untrusted size hint.
pub const MAX_PREALLOC: u64 = 64 * 1024 * 1024;

/// Largest image side we decode.
pub const MAX_IMAGE_SIDE: u32 = 8000;

/// Largest allocation the image decoder may make.
pub const MAX_IMAGE_ALLOC: u64 = 128 * 1024 * 1024;

/// Reads `r` to the end, failing with `InvalidData` once more than `limit` bytes arrive.
/// `size_hint` (e.g. a zip entry's declared size) only sizes the initial buffer and is capped,
/// so a forged header can neither abort the process nor bypass the limit.
pub fn read_limited<R: Read>(r: R, limit: u64, size_hint: u64) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size_hint.min(limit).min(MAX_PREALLOC) as usize);
    r.take(limit.saturating_add(1)).read_to_end(&mut out)?;
    if out.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("data exceeds the limit of {limit} bytes"),
        ));
    }
    Ok(out)
}

fn limits() -> Limits {
    let mut l = Limits::default();
    l.max_image_width = Some(MAX_IMAGE_SIDE);
    l.max_image_height = Some(MAX_IMAGE_SIDE);
    l.max_alloc = Some(MAX_IMAGE_ALLOC);
    l
}

/// Image dimensions from the header, `None` when unknown, empty or above [`MAX_IMAGE_SIDE`].
pub fn checked_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let (w, h) = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    (w > 0 && h > 0 && w <= MAX_IMAGE_SIDE && h <= MAX_IMAGE_SIDE).then_some((w, h))
}

/// Decodes an untrusted image with dimension and allocation limits (header checked first).
pub fn decode_image(data: &[u8]) -> Option<DynamicImage> {
    checked_dimensions(data)?;
    let mut r = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    r.limits(limits());
    r.decode().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_read() {
        let data = vec![7u8; 1000];
        assert_eq!(read_limited(&data[..], 1000, u64::MAX).unwrap().len(), 1000);
        assert!(read_limited(&data[..], 999, 10).is_err());
    }

    #[test]
    fn huge_image_refused() {
        // a PNG header claiming 60000 x 60000 pixels
        let img = image::RgbImage::new(1, 1);
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png[16..20].copy_from_slice(&60000u32.to_be_bytes());
        png[20..24].copy_from_slice(&60000u32.to_be_bytes());
        assert!(checked_dimensions(&png).is_none());
        assert!(decode_image(&png).is_none());
        let mut ok = Vec::new();
        image::RgbImage::new(4, 3)
            .write_to(&mut Cursor::new(&mut ok), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(decode_image(&ok).unwrap().width(), 4);
    }
}
