//! Builders for hostile zip archives, used by the regression tests of every crate
//! (zip bombs and forged entry sizes are generated at runtime, never committed).
#![doc(hidden)]

use flate2::{Compress, Compression, FlushCompress};

/// A raw deflate stream that inflates to `mib` MiB of zeros, built in O(`mib`) time: one
/// sync-flushed block of 1 MiB zeros (self-contained back references) repeated, plus a final
/// empty block.
pub fn deflate_zeros(mib: usize) -> Vec<u8> {
    let zeros = vec![0u8; 1 << 20];
    let mut c = Compress::new(Compression::best(), false);
    let mut chunk = Vec::with_capacity(8 << 10);
    c.compress_vec(&zeros, &mut chunk, FlushCompress::Full)
        .expect("deflate");
    let mut tail = Vec::with_capacity(64);
    c.compress_vec(&[], &mut tail, FlushCompress::Finish)
        .expect("deflate");
    let mut out = Vec::with_capacity(chunk.len() * mib + tail.len());
    for _ in 0..mib {
        out.extend_from_slice(&chunk);
    }
    out.extend_from_slice(&tail);
    out
}

/// A single-entry zip with `data` stored as-is under `method` (0 = stored, 8 = deflate) and a
/// *declared* uncompressed size of `declared_size` (Zip64 extra fields when it needs 64 bits).
/// The CRC is 0, so a reader that gets to the end reports a checksum error.
pub fn raw_zip(name: &str, method: u16, data: &[u8], declared_size: u64) -> Vec<u8> {
    let zip64 = declared_size >= 0xFFFF_FFFF || data.len() as u64 >= 0xFFFF_FFFF;
    let (c32, u32_) = if zip64 {
        (0xFFFF_FFFFu32, 0xFFFF_FFFFu32)
    } else {
        (data.len() as u32, declared_size as u32)
    };
    let mut extra = Vec::new();
    if zip64 {
        extra.extend_from_slice(&1u16.to_le_bytes());
        extra.extend_from_slice(&16u16.to_le_bytes());
        extra.extend_from_slice(&declared_size.to_le_bytes());
        extra.extend_from_slice(&(data.len() as u64).to_le_bytes());
    }
    let version: u16 = if zip64 { 45 } else { 20 };
    let mut z = Vec::with_capacity(data.len() + 256);
    // local file header
    z.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    z.extend_from_slice(&version.to_le_bytes());
    z.extend_from_slice(&0u16.to_le_bytes()); // flags
    z.extend_from_slice(&method.to_le_bytes());
    z.extend_from_slice(&0u16.to_le_bytes()); // time
    z.extend_from_slice(&0x21u16.to_le_bytes()); // date 1980-01-01
    z.extend_from_slice(&0u32.to_le_bytes()); // crc
    z.extend_from_slice(&c32.to_le_bytes());
    z.extend_from_slice(&u32_.to_le_bytes());
    z.extend_from_slice(&(name.len() as u16).to_le_bytes());
    z.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    z.extend_from_slice(name.as_bytes());
    z.extend_from_slice(&extra);
    z.extend_from_slice(data);
    let cd_off = z.len() as u32;
    // central directory
    z.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    z.extend_from_slice(&version.to_le_bytes()); // made by
    z.extend_from_slice(&version.to_le_bytes()); // needed
    z.extend_from_slice(&0u16.to_le_bytes());
    z.extend_from_slice(&method.to_le_bytes());
    z.extend_from_slice(&0u16.to_le_bytes());
    z.extend_from_slice(&0x21u16.to_le_bytes());
    z.extend_from_slice(&0u32.to_le_bytes());
    z.extend_from_slice(&c32.to_le_bytes());
    z.extend_from_slice(&u32_.to_le_bytes());
    z.extend_from_slice(&(name.len() as u16).to_le_bytes());
    z.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    z.extend_from_slice(&0u16.to_le_bytes()); // comment
    z.extend_from_slice(&0u16.to_le_bytes()); // disk
    z.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
    z.extend_from_slice(&0u32.to_le_bytes()); // external attrs
    z.extend_from_slice(&0u32.to_le_bytes()); // local header offset
    z.extend_from_slice(name.as_bytes());
    z.extend_from_slice(&extra);
    let cd_size = z.len() as u32 - cd_off;
    // end of central directory
    z.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    z.extend_from_slice(&0u16.to_le_bytes());
    z.extend_from_slice(&0u16.to_le_bytes());
    z.extend_from_slice(&1u16.to_le_bytes());
    z.extend_from_slice(&1u16.to_le_bytes());
    z.extend_from_slice(&cd_size.to_le_bytes());
    z.extend_from_slice(&cd_off.to_le_bytes());
    z.extend_from_slice(&0u16.to_le_bytes());
    z
}

/// A zip whose only entry inflates to `mib` MiB of zeros but declares `declared_size` bytes.
pub fn zip_bomb(name: &str, mib: usize, declared_size: u64) -> Vec<u8> {
    raw_zip(name, 8, &deflate_zeros(mib), declared_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn bomb_inflates() {
        let z = zip_bomb("x.fb2", 3, 100);
        assert!(z.len() < 64 * 1024);
        let mut za = zip::ZipArchive::new(std::io::Cursor::new(z)).unwrap();
        let f = za.by_index(0).unwrap();
        assert_eq!(f.size(), 100, "declared size is what the zip crate reports");
        let mut n = 0u64;
        let mut buf = [0u8; 65536];
        let mut f = f;
        loop {
            match f.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(k) => n += k as u64,
            }
        }
        assert!(n >= 100, "{n}");
    }
}
