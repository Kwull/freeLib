//! Hostile inputs: zip bombs and forged entry sizes must fail cleanly (no abort, bounded memory).

use freelib_fb2conv::{read_info, read_info_any, read_info_epub, to_kepub};
use freelib_import::testutil::{raw_zip, zip_bomb};

#[test]
fn fb2_zip_bomb() {
    // 257 MiB of zeros declared as 100 bytes: more than the 256 MiB .fb2.zip limit
    let z = zip_bomb("book.fb2", 257, 100);
    assert!(read_info(&z).is_err());
}

#[test]
fn fb2_zip_huge_declared_size() {
    let z = raw_zip("book.fb2", 0, b"<FictionBook/>", 1 << 62);
    assert!(read_info(&z).is_err());
    let z = raw_zip("book.fb2", 0, b"<FictionBook/>", 0xFFFF_FFF0);
    let _ = read_info_any(&z);
}

#[test]
fn epub_container_bomb() {
    let z = zip_bomb("META-INF/container.xml", 65, 10);
    assert!(read_info_epub(&z).is_err());
}

#[test]
fn kepub_bomb() {
    let z = zip_bomb("OEBPS/ch1.xhtml", 257, 10);
    assert!(to_kepub(&z).is_err());
    let z = raw_zip("OEBPS/ch1.xhtml", 0, b"<html/>", 1 << 62);
    let _ = to_kepub(&z);
}
