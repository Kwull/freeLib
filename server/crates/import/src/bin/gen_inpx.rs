//! Generate a synthetic Flibusta-like INPX (and optionally the book archives).
//!
//! ```text
//! gen-inpx --books 600000 --out bench-data/synthetic.inpx [--with-files bench-data/lib] [--per-archive 2000] [--seed 42]
//! ```

use std::path::PathBuf;
use std::time::Instant;

use freelib_import::synth::{GenOptions, generate};

fn usage() -> ! {
    eprintln!(
        "usage: gen-inpx --books N --out FILE.inpx [--with-files DIR] [--per-archive N] [--seed N] [--no-structure]"
    );
    std::process::exit(2)
}

fn main() {
    let mut opts = GenOptions {
        showcase: true,
        books: 10_000,
        ..Default::default()
    };
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--books" => opts.books = val().parse().unwrap_or_else(|_| usage()),
            "--out" => out = Some(PathBuf::from(val())),
            "--with-files" => opts.files_dir = Some(PathBuf::from(val())),
            "--per-archive" => opts.per_archive = val().parse().unwrap_or_else(|_| usage()),
            "--seed" => opts.seed = val().parse().unwrap_or_else(|_| usage()),
            "--no-structure" => opts.structure_info = false,
            _ => usage(),
        }
    }
    let out = out.unwrap_or_else(|| usage());
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let t = Instant::now();
    match generate(&out, &opts) {
        Ok(s) => println!(
            "{}: {} books in {} parts, author pool {}, {} MB of book files, {:.1}s",
            out.display(),
            s.books,
            s.parts,
            s.author_pool,
            s.file_bytes / 1_000_000,
            t.elapsed().as_secs_f64()
        ),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
