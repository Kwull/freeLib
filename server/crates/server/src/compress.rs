//! Compression of cached bodies (the authors/series lists are compressed once per catalog version).

use std::io::Write;

pub fn brotli(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 6);
    {
        let mut w = brotli::CompressorWriter::new(&mut out, 64 * 1024, 7, 22);
        let _ = w.write_all(data);
    }
    out
}

pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut w = flate2::write::GzEncoder::new(Vec::with_capacity(data.len() / 5), flate2::Compression::new(6));
    let _ = w.write_all(data);
    w.finish().unwrap_or_default()
}
