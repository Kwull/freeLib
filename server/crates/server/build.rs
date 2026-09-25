//! Points rust-embed at `web/dist`, or at an empty folder when the web app is not built.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest.join("../../../web/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-env-changed=FREELIB_WEB_DIST");
    let dir = if let Ok(d) = std::env::var("FREELIB_WEB_DIST") {
        PathBuf::from(d)
    } else if dist.is_dir() {
        dist.canonicalize().unwrap_or(dist)
    } else {
        let empty = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("empty-dist");
        std::fs::create_dir_all(&empty).unwrap();
        empty
    };
    println!("cargo:rustc-env=FREELIB_WEB_DIST={}", dir.display());
}
