//! Compression of cached bodies (the authors/series lists are compressed once per catalog version).
//! Brotli quality 5 / window 2^20: on the 6.7 MB authors list of a 600k-book catalog it takes
//! ~160 ms for 1.02 MB (quality 7 / 2^22: ~390 ms for 0.97 MB).

use std::io::Write;

pub fn brotli(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 6);
    {
        let mut w = brotli::CompressorWriter::new(&mut out, 64 * 1024, 5, 20);
        let _ = w.write_all(data);
    }
    out
}

pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut w = flate2::write::GzEncoder::new(
        Vec::with_capacity(data.len() / 5),
        flate2::Compression::new(6),
    );
    let _ = w.write_all(data);
    w.finish().unwrap_or_default()
}
