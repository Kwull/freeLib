//! Input decoding: zipped FB2, BOMs, XML-declared encodings, base64.

use std::borrow::Cow;
use std::io::Cursor;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1251};

use crate::{Error, Result};

/// Maximum uncompressed size accepted from a `.fb2.zip` (guards against zip bombs).
const MAX_UNZIPPED: u64 = 256 * 1024 * 1024;

pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// If `bytes` is a zip archive, returns the first `.fb2` entry (or the first file entry).
pub fn unzip_fb2(bytes: &[u8]) -> Result<Cow<'_, [u8]>> {
    if !is_zip(bytes) {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Zip(e.to_string()))?;
    let mut idx = None;
    for i in 0..zip.len() {
        let Some(name) = zip.name_for_index(i) else {
            continue;
        };
        if name.ends_with('/') {
            continue;
        }
        if name.to_ascii_lowercase().ends_with(".fb2") {
            idx = Some(i);
            break;
        }
        if idx.is_none() {
            idx = Some(i);
        }
    }
    let idx = idx.ok_or_else(|| Error::Format("empty zip archive".into()))?;
    let f = zip.by_index(idx).map_err(|e| Error::Zip(e.to_string()))?;
    if f.size() > MAX_UNZIPPED {
        return Err(Error::Format("zipped file too large".into()));
    }
    let hint = f.size();
    let out = crate::limit::read_limited(f, MAX_UNZIPPED, hint)
        .map_err(|_| Error::Format("zipped file too large".into()))?;
    Ok(Cow::Owned(out))
}

/// Finds the `encoding="…"` label in an ASCII-compatible XML declaration.
fn declared_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    let head = &bytes[..bytes.len().min(256)];
    if !head.starts_with(b"<?xml") {
        // Some files have leading whitespace before the declaration.
        let trimmed = head.iter().position(|b| !b.is_ascii_whitespace())?;
        if !head[trimmed..].starts_with(b"<?xml") {
            return None;
        }
    }
    let end = head.windows(2).position(|w| w == b"?>")?;
    let decl = std::str::from_utf8(&head[..end]).ok()?;
    let i = decl.find("encoding")?;
    let rest = decl[i + 8..].trim_start().strip_prefix('=')?.trim_start();
    let q = rest.chars().next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let rest = &rest[1..];
    let label = &rest[..rest.find(q)?];
    Encoding::for_label(label.trim().as_bytes())
}

/// Decodes an XML document to UTF-8, honouring BOMs and the XML declaration.
/// Invalid UTF-8 with no (or a UTF-8) declaration falls back to windows-1251, which is by far
/// the most common mislabelled encoding in Russian FB2 collections.
pub fn decode_xml(bytes: &[u8]) -> Cow<'_, str> {
    if let Some((enc, bom_len)) = Encoding::for_bom(bytes) {
        let (s, _) = enc.decode_without_bom_handling(&bytes[bom_len..]);
        return s;
    }
    // UTF-16 without BOM: "<\0?\0" or "\0<\0?"
    if bytes.len() >= 4 {
        if bytes[0] == b'<' && bytes[1] == 0 {
            return UTF_16LE.decode_without_bom_handling(bytes).0;
        }
        if bytes[0] == 0 && bytes[1] == b'<' {
            return UTF_16BE.decode_without_bom_handling(bytes).0;
        }
    }
    let enc = declared_encoding(bytes).unwrap_or(UTF_8);
    // UTF-16 declared but no BOM and ASCII-looking bytes: the declaration lies.
    let enc = if enc == UTF_16LE || enc == UTF_16BE {
        UTF_8
    } else {
        enc
    };
    if enc == UTF_8 {
        match std::str::from_utf8(bytes) {
            Ok(s) => return Cow::Borrowed(s),
            Err(e) => {
                // A handful of broken sequences: decode lossily; otherwise assume cp1251.
                let bad = count_invalid_utf8(bytes, e.valid_up_to());
                if bad * 200 < bytes.len() {
                    return String::from_utf8_lossy(bytes);
                }
                return WINDOWS_1251.decode_without_bom_handling(bytes).0;
            }
        }
    }
    enc.decode_without_bom_handling(bytes).0
}

fn count_invalid_utf8(bytes: &[u8], mut pos: usize) -> usize {
    let mut bad = 0;
    while pos < bytes.len() {
        match std::str::from_utf8(&bytes[pos..]) {
            Ok(_) => break,
            Err(e) => {
                bad += 1;
                pos += e.valid_up_to() + e.error_len().unwrap_or(1);
                if bad > 10_000 {
                    break;
                }
            }
        }
    }
    bad
}

/// Lenient base64 decoder: skips whitespace and any non-alphabet characters, accepts both the
/// standard and URL-safe alphabets, and tolerates missing padding.
pub fn base64_decode(s: &str) -> Vec<u8> {
    const INV: u8 = 0xff;
    static TABLE: [u8; 256] = {
        let mut t = [INV; 256];
        let alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0;
        while i < 64 {
            t[alpha[i] as usize] = i as u8;
            i += 1;
        }
        t[b'-' as usize] = 62;
        t[b'_' as usize] = 63;
        t
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut n = 0;
    for &b in s.as_bytes() {
        if b == b'=' {
            break;
        }
        let v = TABLE[b as usize];
        if v == INV {
            continue;
        }
        acc = (acc << 6) | v as u32;
        n += 1;
        if n == 4 {
            out.push((acc >> 16) as u8);
            out.push((acc >> 8) as u8);
            out.push(acc as u8);
            acc = 0;
            n = 0;
        }
    }
    match n {
        2 => out.push((acc >> 4) as u8),
        3 => {
            out.push((acc >> 10) as u8);
            out.push((acc >> 2) as u8);
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64() {
        assert_eq!(base64_decode("aGVs\nbG8gd29y bGQ="), b"hello world");
        assert_eq!(base64_decode("aGVsbG8"), b"hello");
    }

    #[test]
    fn encodings() {
        let (cp, _, _) =
            WINDOWS_1251.encode("<?xml version=\"1.0\" encoding=\"windows-1251\"?><a>Привет</a>");
        assert!(decode_xml(&cp).contains("Привет"));
        let (k, _, _) =
            encoding_rs::KOI8_R.encode("<?xml version='1.0' encoding='koi8-r'?><a>Привет</a>");
        assert!(decode_xml(&k).contains("Привет"));
        // mislabelled cp1251 declared as utf-8
        let (cp, _, _) = WINDOWS_1251
            .encode("<?xml version=\"1.0\" encoding=\"utf-8\"?><a>Привет мир, как дела</a>");
        assert!(decode_xml(&cp).contains("Привет"));
        let mut u16 = vec![0xff, 0xfe];
        for c in "<a>Привет</a>".encode_utf16() {
            u16.extend_from_slice(&c.to_le_bytes());
        }
        assert!(decode_xml(&u16).contains("Привет"));
        let mut u8bom = vec![0xef, 0xbb, 0xbf];
        u8bom.extend_from_slice("<a>Привет</a>".as_bytes());
        assert_eq!(decode_xml(&u8bom), "<a>Привет</a>");
    }
}
