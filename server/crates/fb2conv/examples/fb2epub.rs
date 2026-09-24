//! Converts FB2 files to EPUB and reports timings.
//!
//! ```text
//! cargo run --release -p freelib-fb2conv --example fb2epub -- [--kepub] [--options '<json>'] in.fb2 [out.epub]
//! cargo run --release -p freelib-fb2conv --example fb2epub -- --synthetic 500 out.epub
//! ```

use std::time::Instant;

use freelib_fb2conv::{Assets, ConvertOptions, fb2_to_epub, read_info, to_kepub};

/// Builds a synthetic FB2 of roughly `kb` kilobytes (Russian prose, notes, a poem, an image).
fn synthetic(kb: usize) -> Vec<u8> {
    // pseudo-random words from Russian syllables: lots of unique words, like real prose
    const SYL: [&str; 24] = ["ра", "мо", "ски", "вер", "ло", "да", "не", "при", "сто", "ван", "ка", "ли", "тель", "ность", "про", "же", "ни", "ма", "об", "ре", "ту", "зна", "ще", "го"];
    let mut seed: u64 = 42;
    let mut rnd = move |n: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    let mut sentence = move || {
        let mut s = String::new();
        let words = 6 + rnd(10);
        for w in 0..words {
            let mut word = String::new();
            for _ in 0..1 + rnd(4) {
                word.push_str(SYL[rnd(SYL.len() as u64) as usize]);
            }
            if w == 0 {
                let mut c = word.chars();
                let first = c.next().unwrap().to_uppercase().collect::<String>();
                word = first + c.as_str();
            }
            s.push_str(&word);
            s.push_str(if w + 1 == words { ". " } else if rnd(8) == 0 { ", " } else { " " });
        }
        s
    };
    let mut body = String::new();
    let mut notes = String::new();
    let mut ch = 0;
    while body.len() < kb * 1024 {
        ch += 1;
        body.push_str(&format!("<section id=\"c{ch}\"><title><p>Глава {ch}</p></title>\n"));
        body.push_str("<epigraph><p>Эпиграф к главе.</p><text-author>Автор</text-author></epigraph>\n");
        for p in 0..40 {
            body.push_str("<p>");
            for _ in 0..4 {
                body.push_str(&sentence());
            }
            if p % 10 == 0 {
                body.push_str(&format!("<a l:href=\"#n{ch}_{p}\" type=\"note\">[{p}]</a>"));
                notes.push_str(&format!("<section id=\"n{ch}_{p}\"><title><p>{p}</p></title><p>Примечание {ch}.{p}.</p></section>\n"));
            }
            body.push_str(" <emphasis>Курсив</emphasis> и <strong>полужирный</strong>.</p>\n");
        }
        body.push_str("<poem><stanza><v>Строка стиха первая,</v><v>строка стиха вторая.</v></stanza></poem>\n");
        body.push_str("</section>\n");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<FictionBook xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\" xmlns:l=\"http://www.w3.org/1999/xlink\">\
<description><title-info><genre>sf</genre><author><first-name>Тест</first-name><last-name>Тестов</last-name></author><book-title>Синтетическая книга</book-title><lang>ru</lang></title-info></description>\
<body><title><p>Синтетическая книга</p></title>{body}</body><body name=\"notes\">{notes}</body></FictionBook>"
    )
    .into_bytes()
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut kepub = false;
    let mut opts = ConvertOptions::default();
    let mut input: Option<Vec<u8>> = None;
    while let Some(a) = args.first().cloned() {
        match a.as_str() {
            "--kepub" => {
                kepub = true;
                args.remove(0);
            }
            "--options" => {
                opts = serde_json::from_str(&args[1]).expect("options JSON");
                args.drain(0..2);
            }
            "--synthetic" => {
                input = Some(synthetic(args[1].parse().unwrap()));
                args.drain(0..2);
            }
            _ => break,
        }
    }
    let data = input.unwrap_or_else(|| std::fs::read(&args.remove(0)).expect("input file"));
    let out = args.first().cloned().unwrap_or_else(|| "out.epub".into());
    let assets = Assets::shared();
    // warm up (hyphenation dictionaries, cover background)
    let _ = fb2_to_epub(&data, &opts, assets);
    let t = Instant::now();
    let info = read_info(&data).expect("read_info");
    let t_info = t.elapsed();
    let n = 10;
    let t = Instant::now();
    let mut epub = Vec::new();
    for _ in 0..n {
        epub = fb2_to_epub(&data, &opts, assets).expect("convert");
    }
    let t_conv = t.elapsed() / n;
    let t = Instant::now();
    let k = if kepub { Some(to_kepub(&epub).expect("kepub")) } else { None };
    let t_kepub = t.elapsed();
    eprintln!(
        "{:?}: input {} KB, epub {} KB; read_info {:?}, fb2_to_epub {:?} (avg of {n}), to_kepub {:?}",
        info.title,
        data.len() / 1024,
        epub.len() / 1024,
        t_info,
        t_conv,
        t_kepub
    );
    std::fs::write(&out, k.unwrap_or(epub)).unwrap();
}
