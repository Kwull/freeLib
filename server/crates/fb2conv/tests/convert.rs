use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use freelib_fb2conv::*;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

struct Epub {
    names: Vec<String>,
    files: BTreeMap<String, Vec<u8>>,
    first_stored: bool,
}

impl Epub {
    fn open(bytes: &[u8]) -> Epub {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut files = BTreeMap::new();
        let mut names = Vec::new();
        let mut first_stored = false;
        for i in 0..z.len() {
            let mut f = z.by_index(i).unwrap();
            if i == 0 {
                first_stored = f.compression() == zip::CompressionMethod::Stored
                    && f.extra_data().is_none_or(|e| e.is_empty());
            }
            let mut v = Vec::new();
            f.read_to_end(&mut v).unwrap();
            names.push(f.name().to_string());
            files.insert(f.name().to_string(), v);
        }
        Epub {
            names,
            files,
            first_stored,
        }
    }
    fn text(&self, name: &str) -> String {
        String::from_utf8(
            self.files
                .get(name)
                .unwrap_or_else(|| panic!("missing {name}; have {:?}", self.names))
                .clone(),
        )
        .unwrap()
    }
    fn has(&self, name: &str) -> bool {
        self.files.contains_key(name)
    }
    fn xhtml(&self) -> Vec<(String, String)> {
        self.files
            .iter()
            .filter(|(k, _)| k.ends_with(".xhtml"))
            .map(|(k, v)| (k.clone(), String::from_utf8(v.clone()).unwrap()))
            .collect()
    }
    /// All content documents concatenated in spine order-ish (by name).
    fn all_text(&self) -> String {
        self.xhtml()
            .into_iter()
            .map(|(_, v)| v)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Strict well-formedness check + unique ids.
fn check_xml(name: &str, src: &str) {
    let mut r = quick_xml::Reader::from_str(src);
    let mut depth = 0i32;
    loop {
        match r.read_event() {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(quick_xml::events::Event::Start(_)) => depth += 1,
            Ok(quick_xml::events::Event::End(_)) => depth -= 1,
            Ok(_) => {}
            Err(e) => panic!(
                "{name}: not well-formed at {}: {e}\n{src}",
                r.buffer_position()
            ),
        }
    }
    assert_eq!(depth, 0, "{name}: unbalanced");
    let mut ids = HashSet::new();
    for part in src.split(" id=\"").skip(1) {
        let id = &part[..part.find('"').unwrap()];
        assert!(!id.is_empty(), "{name}: empty id");
        assert!(ids.insert(id.to_string()), "{name}: duplicate id {id}");
    }
}

fn check_epub_structure(e: &Epub) {
    assert_eq!(e.names[0], "mimetype");
    assert!(
        e.first_stored,
        "mimetype must be stored without extra fields"
    );
    assert_eq!(e.text("mimetype"), "application/epub+zip");
    assert!(e.has("META-INF/container.xml"));
    for (k, v) in &e.files {
        if k.ends_with(".xhtml")
            || k.ends_with(".opf")
            || k.ends_with(".ncx")
            || k.ends_with(".xml")
        {
            check_xml(k, std::str::from_utf8(v).unwrap());
        }
    }
    for (k, v) in e.xhtml() {
        assert!(v.contains("xmlns=\"http://www.w3.org/1999/xhtml\""), "{k}");
        assert!(v.contains(" lang=\"") && v.contains(" xml:lang=\""), "{k}");
    }
    // every manifest item exists
    let opf = e.text("OEBPS/content.opf");
    for part in opf.split("<item ").skip(1) {
        let href = part
            .split("href=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(
            e.has(&format!("OEBPS/{href}")),
            "manifest item {href} missing"
        );
    }
}

// ------------------------------------------------------------------------------------ epubcheck

fn epubcheck_jar() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EPUBCHECK_JAR") {
        return Some(PathBuf::from(p));
    }
    let bench = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench-data");
    let dir = std::fs::read_dir(&bench).ok()?;
    for d in dir.flatten() {
        let jar = d.path().join("epubcheck.jar");
        if d.file_name().to_string_lossy().starts_with("epubcheck") && jar.exists() {
            return Some(jar);
        }
    }
    None
}

fn java_available() -> bool {
    Command::new("java")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Runs epubcheck; panics on errors. Returns false when epubcheck is unavailable.
fn epubcheck(name: &str, bytes: &[u8]) -> bool {
    let Some(jar) = epubcheck_jar() else {
        eprintln!(
            "epubcheck not found (set EPUBCHECK_JAR or put it into server/bench-data) — skipping"
        );
        return false;
    };
    if !java_available() {
        eprintln!("java not available — skipping epubcheck");
        return false;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    let out = Command::new("java")
        .arg("-jar")
        .arg(&jar)
        .arg(&path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let problems: Vec<&str> = stdout
        .lines()
        .chain(stderr.lines())
        .filter(|l| l.starts_with("ERROR") || l.starts_with("FATAL") || l.starts_with("WARNING"))
        .collect();
    assert!(
        problems.is_empty() && out.status.success(),
        "epubcheck {name}:\n{stdout}\n{stderr}"
    );
    true
}

// ------------------------------------------------------------------------------------ read_info

#[test]
fn info_full() {
    let i = read_info(&fixture("full.fb2")).unwrap();
    assert_eq!(i.title, "Трудно быть богом");
    assert_eq!(i.authors.len(), 2);
    assert_eq!(i.authors[0].display_name(), "Стругацкий Аркадий Натанович");
    assert_eq!(i.series.as_deref(), Some("Мир Полудня"));
    assert_eq!(i.serno, Some(3));
    assert_eq!(i.genres, vec!["sf_social", "sf_epic"]);
    assert_eq!(i.lang, "ru");
    assert_eq!(i.date, "1964");
    assert_eq!(i.publisher, "Детская литература");
    assert_eq!(i.isbn, "978-5-17-000000-0");
    assert_eq!(i.keywords, "фантастика, прогрессоры");
    assert_eq!(
        i.annotation.as_deref(),
        Some(
            "<p>Роман о <em>земном</em> историке, <strong>работающем</strong> на планете &lt;Арканар&gt;.</p><p>Вторая строка аннотации &amp; ещё.</p>"
        )
    );
    let c = i.cover.unwrap();
    assert_eq!(c.mime, "image/jpeg");
    assert_eq!(&c.data[..3], &[0xff, 0xd8, 0xff]);
    // JSON shape used for caching
    let j = serde_json::to_value(read_info(&fixture("full.fb2")).unwrap()).unwrap();
    assert_eq!(j["serno"], 3);
    assert_eq!(j["authors"][0]["last"], "Стругацкий");
    assert!(j.get("cover").is_none());
}

#[test]
fn info_encodings() {
    let i = read_info(&fixture("cp1251.fb2")).unwrap();
    assert_eq!(i.title, "Хаджи-Мурат");
    assert_eq!(i.authors[0].display_name(), "Толстой Лев");
    assert_eq!(
        i.annotation.as_deref(),
        Some("<p>Повесть о «Хаджи-Мурате».</p>")
    );
    assert!(i.cover.is_none(), "broken cover must be ignored");

    let i = read_info(&fixture("koi8.fb2.zip")).unwrap();
    assert_eq!(i.title, "Каштанка");
    assert_eq!(i.series.as_deref(), Some("Рассказы"));
    assert_eq!(i.serno, Some(7));

    let i = read_info(&fixture("utf16.fb2")).unwrap();
    assert_eq!(i.title, "Кобзар");
    assert_eq!(i.lang, "uk");

    let i = read_info(&fixture("broken.fb2")).unwrap();
    assert_eq!(i.title, "Broken & Markup");
    assert_eq!(i.authors[0].last, "Broken");

    assert!(read_info(b"<html><body>no</body></html>").is_err());
    assert!(read_info(b"garbage").is_err());
}

// ------------------------------------------------------------------------------------ conversion

#[test]
fn convert_full_default() {
    let opts = ConvertOptions::default();
    let bytes = fb2_to_epub(&fixture("full.fb2"), &opts, Assets::shared()).unwrap();
    let e = Epub::open(&bytes);
    check_epub_structure(&e);
    let opf = e.text("OEBPS/content.opf");
    assert!(opf.contains("version=\"3.0\""));
    assert!(opf.contains(
        "<dc:identifier id=\"bookid\">urn:uuid:6f9619ff-8b86-d011-b42d-00c04fc964ff</dc:identifier>"
    ));
    assert!(opf.contains("<dc:title id=\"title\">Трудно быть богом</dc:title>"));
    assert!(opf.contains("<dc:creator id=\"creator1\">Аркадий Натанович Стругацкий</dc:creator>"));
    assert!(opf.contains("property=\"file-as\">Стругацкий, Аркадий Натанович</meta>"));
    assert!(opf.contains("property=\"dcterms:modified\""));
    assert!(opf.contains("properties=\"cover-image\""));
    assert!(opf.contains("<meta name=\"cover\" content=\"cover-image\"/>"));
    assert!(
        opf.contains("<meta property=\"belongs-to-collection\" id=\"series\">Мир Полудня</meta>")
    );
    assert!(opf.contains("<meta refines=\"#series\" property=\"group-position\">3</meta>"));
    assert!(opf.contains("properties=\"nav\""));
    assert!(opf.contains("<spine toc=\"ncx\">"));
    assert!(opf.contains("<dc:date>1964</dc:date>"));
    // spine: cover, annotation, title, ..., notes, nav (end)
    let spine: Vec<&str> = opf
        .split("<itemref idref=\"")
        .skip(1)
        .map(|s| s.split('"').next().unwrap())
        .collect();
    assert_eq!(spine.last(), Some(&"nav"));
    assert!(
        opf.contains(
            "href=\"cover.xhtml\" media-type=\"application/xhtml+xml\" properties=\"svg\""
        )
    );

    let cover = e.text("OEBPS/cover.xhtml");
    assert!(
        cover.contains("<svg")
            && cover.contains("xlink:href=\"img/cover.jpg\"")
            && cover.contains("viewBox=\"0 0 300 450\"")
    );
    assert!(e.has("OEBPS/img/cover.jpg"));

    let ann = e.text("OEBPS/annotation.xhtml");
    assert!(ann.contains("Аннотация") && ann.contains("<em>земном</em>"));

    let title = e.text("OEBPS/title.xhtml");
    assert!(title.contains("<h1 class=\"titleblock h0\""));
    assert!(title.contains("Аркадий и Борис Стругацкие<br/>Трудно быть богом"));
    assert!(title.contains("<div class=\"epigraph\">"));
    assert!(title.contains("<p class=\"text-author\">Пьер Абеляр</p>"));

    let all = e.all_text();
    // headings: prologue (depth 1), part (1), chapters (2)
    assert!(
        all.contains("<h2 class=\"titleblock h1\" id=\"prologue\">Пролог</h2>"),
        "{all}"
    );
    assert!(all.contains("<h3 class=\"titleblock h2\" id=\"chapter1\">Глава 1<br/>Румата</h3>"));
    // inline styles
    assert!(all.contains("<em>тяжёлый</em>") && all.contains("<strong>толстой</strong>"));
    assert!(all.contains("<span class=\"strike\">зачёркиванием</span>"));
    assert!(
        all.contains("H<sub>2</sub>O")
            && all.contains("x<sup>2</sup>")
            && all.contains("<code>code()</code>")
    );
    assert!(all.contains("<p class=\"subtitle\">* * *</p>"));
    assert!(all.contains("<p class=\"empty-line\">"));
    // dialogue dash keeps a no-break space
    assert!(all.contains("<p>—\u{a0}Здравствуй"));
    // links
    assert!(all.contains("<a href=\"http://example.com/?a=1&amp;b=2\">сайт</a>"));
    assert!(
        all.contains("<a>несуществующее</a>"),
        "unknown targets lose href"
    );
    let chapter2_file = e
        .xhtml()
        .into_iter()
        .find(|(_, v)| v.contains("id=\"chapter2\""))
        .unwrap()
        .0;
    let chapter2_file = chapter2_file.trim_start_matches("OEBPS/");
    assert!(
        all.contains(&format!(
            "<a href=\"{chapter2_file}#chapter2\">вторую главу</a>"
        )),
        "{all}"
    );
    // poem, cite, table
    assert!(all.contains("<div class=\"poem\" id=\"poem1\">"));
    assert!(all.contains("<p class=\"subtitle\">Стихотворение</p>"));
    assert!(all.contains("<div class=\"stanza\">\n<p class=\"v\">Первая строка стиха,</p>"));
    assert!(all.contains("<p class=\"date\">1963</p>"));
    assert!(all.contains("<blockquote class=\"cite\" id=\"cite1\">"));
    assert!(all.contains("<table class=\"table\">"));
    assert!(all.contains("<th style=\"text-align: center;\">Роль</th>"));
    assert!(all.contains("<td style=\"text-align: right;vertical-align: top;\">Дон</td>"));
    assert!(all.contains("<td colspan=\"2\">Общая ячейка</td>"));
    assert!(all.contains("<div class=\"annotation\">"));
    // images
    assert!(all.contains("<div class=\"image\" id=\"img1\"><img src=\"img/pic.png\" alt=\"Картинка\"/><p class=\"image-title\">Картинка</p></div>"));
    assert!(all.contains("<img class=\"inlineimage\" src=\"img/small.gif\" alt=\"\"/>"));
    assert!(e.has("OEBPS/img/pic.png") && e.has("OEBPS/img/small.gif"));
    assert!(
        e.has("OEBPS/img/pic.jpg"),
        "BMP converted to JPEG: {:?}",
        e.names
    );
    assert!(
        !e.names.iter().any(|n| n.contains("broken")),
        "broken image dropped"
    );
    assert!(!all.contains("nonexistent"));
    // notes (end): noteref + footnote with backlink
    assert!(all.contains("<a class=\"anchor\" epub:type=\"noteref\" id=\"ref"));
    assert!(all.contains("href=\"notes.xhtml#n1\">[1]</a>"));
    let notes = e.text("OEBPS/notes.xhtml");
    assert!(notes.contains("<h1 class=\"titlenotes\""));
    assert!(notes.contains("Примечания"));
    assert!(notes.contains("<div class=\"note\" epub:type=\"footnote\" id=\"n1\">"));
    assert!(
        notes.contains("<p class=\"note-title\"><a href=\"part0001.xhtml#ref"),
        "{notes}"
    );
    assert!(notes.contains("Первое примечание с <em>выделением</em>."));
    let chapter1_file = e
        .xhtml()
        .into_iter()
        .find(|(_, v)| v.contains("id=\"chapter1\""))
        .unwrap()
        .0;
    let chapter1_file = chapter1_file.trim_start_matches("OEBPS/");
    assert!(
        notes.contains(&format!("<a href=\"{chapter1_file}#chapter1\">главу</a>")),
        "{notes}"
    );
    // unreferenced note still present without a link
    assert!(notes.contains("<p class=\"note-title\">3</p>"));
    // nav & ncx
    let nav = e.text("OEBPS/nav.xhtml");
    assert!(nav.contains("<nav epub:type=\"toc\" id=\"toc\">"));
    assert!(
        nav.contains(">Часть первая</a>\n"),
        "nested list under part: {nav}"
    );
    assert!(nav.contains("#chapter1\">Глава 1 Румата</a>"));
    assert!(nav.contains("epub:type=\"landmarks\""));
    let ncx = e.text("OEBPS/toc.ncx");
    assert!(ncx.contains(
        "<meta name=\"dtb:uid\" content=\"urn:uuid:6f9619ff-8b86-d011-b42d-00c04fc964ff\"/>"
    ));
    assert!(ncx.contains("<meta name=\"dtb:depth\" content=\"2\"/>"));
    // no hyphenation by default
    assert!(!all.contains('\u{ad}'));
    epubcheck("full.epub", &bytes);

    // read back the produced EPUB
    let back = read_info_epub(&bytes).unwrap();
    assert_eq!(back.title, "Трудно быть богом");
    assert_eq!(back.authors[1].display_name(), "Стругацкий Борис Натанович");
    assert_eq!(back.series.as_deref(), Some("Мир Полудня"));
    assert_eq!(back.serno, Some(3));
    assert_eq!(back.lang, "ru");
    assert!(
        back.annotation
            .as_deref()
            .unwrap()
            .starts_with("<p>Роман о земном историке")
    );
    assert_eq!(back.cover.unwrap().mime, "image/jpeg");
    assert_eq!(read_info_any(&bytes).unwrap().title, "Трудно быть богом");
    assert_eq!(
        read_info_any(&fixture("koi8.fb2.zip")).unwrap().title,
        "Каштанка"
    );
}

#[test]
fn convert_options() {
    let opts = ConvertOptions {
        hyphenate: Hyphenate::Soft,
        footnotes: Footnotes::Popup,
        drop_caps: true,
        break_after_chapter: true,
        toc_placement: TocPlacement::Start,
        create_cover: CreateCover::Missing,
        cover_label: Some("%s %n".into()),
        annotation: false,
        font_family: Some("PT Serif".into()),
        user_css: Some("p { line-height: 1.3; }".into()),
        ..Default::default()
    };
    let bytes = fb2_to_epub(&fixture("full.fb2"), &opts, Assets::shared()).unwrap();
    let e = Epub::open(&bytes);
    check_epub_structure(&e);
    let all = e.all_text();
    assert!(all.contains('\u{ad}'), "soft hyphens");
    assert!(
        all.contains("Пе\u{ad}ре\u{ad}во\u{ad}пло\u{ad}ще\u{ad}ние") || all.contains("ре\u{ad}во"),
        "hyphenated word"
    );
    assert!(!e.has("OEBPS/annotation.xhtml"));
    assert!(
        all.replace('\u{ad}', "")
            .contains("<p class=\"dropcaps\"><span class=\"dropcaps\">П</span>ервый абзац"),
        "{all}"
    );
    let notes = e.text("OEBPS/notes.xhtml");
    assert!(notes.contains("<aside class=\"note\" epub:type=\"footnote\" id=\"n1\">"));
    let opf = e.text("OEBPS/content.opf");
    let spine: Vec<&str> = opf
        .split("<itemref idref=\"")
        .skip(1)
        .map(|s| s.split('"').next().unwrap())
        .collect();
    assert_eq!(spine[1], "nav", "toc at start after the cover: {spine:?}");
    assert!(
        e.has("OEBPS/fonts/PTSerif-Regular.ttf")
            && e.has("OEBPS/fonts/PTSerif-BoldItalic.ttf")
            && e.has("OEBPS/fonts/Sangha.ttf")
    );
    assert!(e.has("META-INF/com.apple.ibooks.display-options.xml"));
    let css = e.text("OEBPS/css/main.css");
    assert!(
        css.contains("font-family: \"PT Serif\", serif;")
            && css.contains("p { line-height: 1.3; }")
    );
    // cover label drawn → re-encoded, still a JPEG of the same size
    let cover = &e.files["OEBPS/img/cover.jpg"];
    assert_ne!(
        cover.as_slice(),
        &read_info(&fixture("full.fb2")).unwrap().cover.unwrap().data[..]
    );
    epubcheck("options.epub", &bytes);

    // inline notes, no toc page
    let opts = ConvertOptions {
        footnotes: Footnotes::Inline,
        toc_placement: TocPlacement::None,
        hyphenate: Hyphenate::Full,
        ..Default::default()
    };
    let bytes = fb2_to_epub(&fixture("full.fb2"), &opts, Assets::shared()).unwrap();
    let e = Epub::open(&bytes);
    check_epub_structure(&e);
    let all = e.all_text();
    assert!(!e.has("OEBPS/notes.xhtml"));
    assert!(
        all.replace('\u{ad}', "").contains(
            "тетивой <span class=\"inlinenote\">[Первое примечание с <em>выделением</em>.]</span>."
        ),
        "{all}"
    );
    let opf = e.text("OEBPS/content.opf");
    assert!(!opf.contains("<itemref idref=\"nav\"/>"));
    assert!(e.text("OEBPS/css/main.css").contains("hyphens: auto"));
    epubcheck("inline.epub", &bytes);
}

#[test]
fn convert_cp1251_broken_cover_and_generated() {
    let bytes = fb2_to_epub(
        &fixture("cp1251.fb2"),
        &ConvertOptions::default(),
        Assets::shared(),
    )
    .unwrap();
    let e = Epub::open(&bytes);
    check_epub_structure(&e);
    assert!(
        e.has("OEBPS/img/cover.jpg"),
        "generated cover for a broken one"
    );
    let cover = &e.files["OEBPS/img/cover.jpg"];
    let img = image::load_from_memory(cover).unwrap();
    assert!(img.width() > 500 && img.height() > 800);
    let all = e.all_text();
    assert!(all.contains("Я возвращался домой полями."));
    assert!(all.contains("<p>«Ёлки-палки», — подумал я.</p>"));
    // no body title: title page synthesized from metadata
    let title = e.text("OEBPS/title.xhtml");
    assert!(
        title.contains("<p class=\"tp-author\">Лев Толстой</p>") && title.contains("Хаджи-Мурат")
    );
    epubcheck("cp1251.epub", &bytes);

    // never → no cover at all
    let bytes = fb2_to_epub(
        &fixture("cp1251.fb2"),
        &ConvertOptions {
            create_cover: CreateCover::Never,
            ..Default::default()
        },
        Assets::shared(),
    )
    .unwrap();
    let e = Epub::open(&bytes);
    assert!(!e.has("OEBPS/cover.xhtml") && !e.text("OEBPS/content.opf").contains("cover-image"));
    epubcheck("nocover.epub", &bytes);

    // always → generated even though the FB2 has a cover
    let bytes = fb2_to_epub(
        &fixture("full.fb2"),
        &ConvertOptions {
            create_cover: CreateCover::Always,
            ..Default::default()
        },
        Assets::shared(),
    )
    .unwrap();
    let e = Epub::open(&bytes);
    let img = image::load_from_memory(&e.files["OEBPS/img/cover.jpg"]).unwrap();
    assert!(img.width() > 500);
}

#[test]
fn convert_other_inputs() {
    for (name, needle) in [
        ("broken.fb2", "Chapter\u{a0}One"),
        ("koi8.fb2.zip", "Молодая рыжая собака."),
        ("utf16.fb2", "Реве та стогне Дніпр широкий."),
    ] {
        let opts = ConvertOptions {
            hyphenate: Hyphenate::Soft,
            ..Default::default()
        };
        let bytes = fb2_to_epub(&fixture(name), &opts, Assets::shared()).unwrap();
        let e = Epub::open(&bytes);
        check_epub_structure(&e);
        let all = e.all_text().replace('\u{ad}', "");
        assert!(all.contains(needle), "{name}: {all}");
        epubcheck(&format!("{name}.epub"), &bytes);
    }
    let bytes = fb2_to_epub(
        &fixture("broken.fb2"),
        &ConvertOptions::default(),
        Assets::shared(),
    )
    .unwrap();
    let all = Epub::open(&bytes).all_text();
    assert!(all.contains("stray &amp; ampersand and &amp;unknown; entity."));
    assert!(all.contains("<em>unclosed emphasis"));
    assert!(
        fb2_to_epub(
            b"not xml at all",
            &ConvertOptions::default(),
            Assets::shared()
        )
        .is_err()
    );
}

#[test]
fn join_series() {
    let a = fixture("series1.fb2");
    let b = fixture("series2.fb2");
    let bytes = join_to_epub(
        &[&a, &b],
        &ConvertOptions::default(),
        Assets::shared(),
        None,
    )
    .unwrap();
    let e = Epub::open(&bytes);
    check_epub_structure(&e);
    let opf = e.text("OEBPS/content.opf");
    assert!(opf.contains("<dc:title id=\"title\">Foundation</dc:title>"));
    assert!(
        e.has("OEBPS/series.xhtml") && e.has("OEBPS/title01.xhtml") && e.has("OEBPS/title02.xhtml")
    );
    assert!(e.has("OEBPS/img/b1_pic.png") && e.has("OEBPS/img/b2_pic.png"));
    let all = e.all_text();
    assert!(all.contains("id=\"b1_c1\"") && all.contains("id=\"b2_c1\""));
    assert!(all.contains("#b2_c1\">chapter 1</a>"));
    let notes = e.text("OEBPS/notes.xhtml");
    assert!(
        notes.contains("id=\"b1_n1\"")
            && notes.contains("id=\"b2_n1\"")
            && notes.contains("Foundation and Empire")
    );
    let nav = e.text("OEBPS/nav.xhtml");
    assert!(
        nav.contains(">Foundation and Empire</a>\n"),
        "book entries have nested chapters: {nav}"
    );
    epubcheck("series.epub", &bytes);
}

#[test]
fn kepub() {
    let epub = fb2_to_epub(
        &fixture("full.fb2"),
        &ConvertOptions {
            footnotes: Footnotes::Popup,
            ..Default::default()
        },
        Assets::shared(),
    )
    .unwrap();
    let k = to_kepub(&epub).unwrap();
    let e = Epub::open(&k);
    check_epub_structure(&e);
    let part = e.text("OEBPS/part0001.xhtml");
    assert!(part.contains("<div id=\"book-columns\"><div id=\"book-inner\">"));
    assert!(part.contains("<span class=\"koboSpan\" id=\"kobo."));
    assert!(part.contains("div#book-inner"));
    let nav = e.text("OEBPS/nav.xhtml");
    assert!(!nav.contains("koboSpan"), "nav stays untouched");
    // images are kept byte for byte
    assert_eq!(
        Epub::open(&epub).files["OEBPS/img/pic.png"],
        e.files["OEBPS/img/pic.png"]
    );
    // idempotent
    let k2 = to_kepub(&k).unwrap();
    assert_eq!(Epub::open(&k2).text("OEBPS/part0001.xhtml"), part);
    epubcheck("full.kepub.epub", &k);
}

#[test]
fn file_names_and_fonts() {
    let f = NameFields {
        author_last: "Азимов".into(),
        author_first: "Айзек".into(),
        series: Some("Основание".into()),
        serno: Some(2),
        title: "Основание и Империя".into(),
        lang: "ru".into(),
        date: "1952".into(),
        ..Default::default()
    };
    assert_eq!(
        file_name("%a/%s/%n %b", &f, false),
        "Азимов А/Основание/02 Основание и Империя"
    );
    assert_eq!(
        file_name("%a - %b", &f, true),
        "Azimov A. - Osnovanie i Imperija"
    );
    assert_eq!(expand_template("%s %n", &f), "Основание 02");
    assert_eq!(Assets::shared().font_names(), vec!["PT Serif".to_string()]);
    assert_eq!(
        serde_json::to_value(ConvertOptions::default()).unwrap()["tocPlacement"],
        "end"
    );
    let o: ConvertOptions = serde_json::from_str(
        r#"{"hyphenate":"soft","footnotes":"popup","dropCaps":true,"fontFamily":null}"#,
    )
    .unwrap();
    assert_eq!(o.hyphenate, Hyphenate::Soft);
    assert!(o.drop_caps && o.break_after_chapter);
}

#[test]
fn splitting() {
    // nested chapters stay in the part's file without breakAfterChapter
    let opts = ConvertOptions {
        break_after_chapter: false,
        ..Default::default()
    };
    let e = Epub::open(&fb2_to_epub(&fixture("full.fb2"), &opts, Assets::shared()).unwrap());
    let part = e
        .xhtml()
        .into_iter()
        .find(|(_, v)| v.contains(">Часть первая</h2>"))
        .unwrap()
        .1;
    assert!(part.contains("id=\"chapter1\"") && part.contains("id=\"chapter2\""));
    let opts = ConvertOptions::default();
    let e = Epub::open(&fb2_to_epub(&fixture("full.fb2"), &opts, Assets::shared()).unwrap());
    let part = e
        .xhtml()
        .into_iter()
        .find(|(_, v)| v.contains(">Часть первая</h2>"))
        .unwrap()
        .1;
    assert!(!part.contains("id=\"chapter1\""));

    // one huge section without sub-sections is split by size between paragraphs
    let mut body = String::from("<section><title><p>Big</p></title>");
    for i in 0..6000 {
        body.push_str(&format!("<p id=\"p{i}\">Paragraph number {i} with some text to make it long enough for the size limit.</p>"));
    }
    body.push_str("<p><a l:href=\"#p1\">back to the start</a></p></section>");
    let fb2 = format!(
        "<?xml version=\"1.0\"?><FictionBook xmlns:l=\"http://www.w3.org/1999/xlink\"><description><title-info><book-title>Big</book-title><lang>en</lang></title-info></description><body>{body}</body></FictionBook>"
    );
    let bytes = fb2_to_epub(
        fb2.as_bytes(),
        &ConvertOptions {
            create_cover: CreateCover::Never,
            ..Default::default()
        },
        Assets::shared(),
    )
    .unwrap();
    let e = Epub::open(&bytes);
    check_epub_structure(&e);
    let parts: Vec<_> = e
        .xhtml()
        .into_iter()
        .filter(|(k, _)| k.contains("part"))
        .collect();
    assert!(parts.len() >= 2, "{:?}", e.names);
    assert!(parts.iter().all(|(_, v)| v.len() < 400 * 1024));
    assert!(
        parts
            .last()
            .unwrap()
            .1
            .contains("<a href=\"part0001.xhtml#p1\">back to the start</a>")
    );
    epubcheck("big.epub", &bytes);
}

// ------------------------------------------------------------------------- catalog metadata

type MetaEl = (String, Vec<(String, String)>, String);

/// Elements of the OPF `<metadata>`: (name, attributes, text).
fn opf_metadata(opf: &str) -> Vec<MetaEl> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(opf);
    let mut out = Vec::new();
    let mut in_meta = false;
    let mut cur: Option<MetaEl> = None;
    loop {
        match r.read_event().unwrap() {
            Event::Start(e) if e.name().as_ref() == "metadata" => in_meta = true,
            Event::Start(e) | Event::Empty(e) if in_meta => {
                let name = e.name().as_ref().to_string();
                let attrs = e
                    .attributes()
                    .flatten()
                    .map(|a| {
                        (
                            a.key.as_ref().to_string(),
                            a.normalized_value(quick_xml::XmlVersion::default())
                                .unwrap()
                                .into_owned(),
                        )
                    })
                    .collect();
                if let Some(c) = cur.take() {
                    out.push(c);
                }
                cur = Some((name, attrs, String::new()));
            }
            Event::Text(t) => {
                if let Some(c) = cur.as_mut() {
                    c.2.push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(g) => {
                if let Some(c) = cur.as_mut() {
                    let name = g.to_string();
                    c.2.push_str(match name.as_str() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        _ => "?",
                    });
                }
            }
            Event::End(e) if e.name().as_ref() == "metadata" => {
                if let Some(c) = cur.take() {
                    out.push(c);
                }
                in_meta = false;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

fn meta_value(m: &[MetaEl], pred: impl Fn(&[(String, String)], &str) -> bool) -> Option<String> {
    m.iter().find(|(n, a, _)| pred(a, n)).map(|(_, a, t)| {
        a.iter()
            .find(|(k, _)| k == "content")
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| t.trim().to_string())
    })
}

fn attr<'a>(a: &'a [(String, String)], k: &str) -> Option<&'a str> {
    a.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str())
}

fn refined(m: &[MetaEl], id: &str, prop: &str) -> Option<String> {
    meta_value(m, |a, _| {
        attr(a, "refines") == Some(id) && attr(a, "property") == Some(prop)
    })
}

fn named(m: &[MetaEl], name: &str) -> Option<String> {
    meta_value(m, |a, _| attr(a, "name") == Some(name))
}

fn element(m: &[MetaEl], name: &str) -> Option<String> {
    meta_value(m, |_, n| n == name)
}

#[test]
fn catalog_metadata_series_and_cover() {
    // an FB2 without <lang>, with junk in the title, a repeated series number and no cover
    let src = String::from_utf8(fixture("series1.fb2"))
        .unwrap()
        .replace("<lang>en</lang>", "")
        .replace(
            "<book-title>Foundation</book-title>",
            "<book-title>  The   Foundation (fb2). Book 3 </book-title><date value=\"1951-06-01\">1951-06-01</date>",
        )
        .replace("<sequence name=\"Foundation\" number=\"1\"/>", "")
        .replace(
            "</title-info>",
            "<annotation><p>First <emphasis>line</emphasis> &amp; more.</p><p>Second.</p></annotation></title-info><publish-info><publisher>Gnome Press</publisher><isbn>978-0-553-29335-7</isbn></publish-info>",
        );
    let meta = BookMeta {
        book_key: Some("lib:4242".into()),
        lang: Some("eng".into()),
        series: Some("Foundation".into()),
        serno: Some(3),
    };
    let opts = ConvertOptions::default();
    let bytes = fb2_to_epub_with(src.as_bytes(), &opts, Assets::shared(), &meta).unwrap();
    let again = fb2_to_epub_with(src.as_bytes(), &opts, Assets::shared(), &meta).unwrap();
    let e = Epub::open(&bytes);
    check_epub_structure(&e);
    let opf = e.text("OEBPS/content.opf");
    let m = opf_metadata(&opf);
    let uid = book_uuid("lib:4242");
    assert_eq!(
        meta_value(&m, |a, n| n == "dc:identifier"
            && attr(a, "id") == Some("bookid"))
        .as_deref(),
        Some(uid.as_str())
    );
    assert!(
        Epub::open(&again).text("OEBPS/content.opf").contains(&uid),
        "re-conversion keeps the identifier"
    );
    assert_eq!(element(&m, "dc:language").as_deref(), Some("en"));
    assert_eq!(element(&m, "dc:title").as_deref(), Some("The Foundation"));
    assert_eq!(
        refined(&m, "#title", "file-as").as_deref(),
        Some("Foundation, The")
    );
    assert_eq!(
        named(&m, "calibre:title_sort").as_deref(),
        Some("Foundation, The")
    );
    assert_eq!(
        refined(&m, "#creator1", "file-as").as_deref(),
        Some("Asimov, Isaac")
    );
    assert_eq!(
        meta_value(&m, |a, _| attr(a, "property")
            == Some("belongs-to-collection"))
        .as_deref(),
        Some("Foundation")
    );
    assert_eq!(
        refined(&m, "#series", "collection-type").as_deref(),
        Some("series")
    );
    assert_eq!(
        refined(&m, "#series", "group-position").as_deref(),
        Some("3")
    );
    assert_eq!(named(&m, "calibre:series").as_deref(), Some("Foundation"));
    assert_eq!(named(&m, "calibre:series_index").as_deref(), Some("3"));
    assert_eq!(
        element(&m, "dc:description").as_deref(),
        Some("First line & more. Second.")
    );
    assert_eq!(element(&m, "dc:publisher").as_deref(), Some("Gnome Press"));
    assert_eq!(element(&m, "dc:date").as_deref(), Some("1951-06-01"));
    assert_eq!(
        meta_value(&m, |a, n| n == "dc:identifier"
            && attr(a, "id") == Some("isbn"))
        .as_deref(),
        Some("urn:isbn:9780553293357")
    );
    // generated cover: a device-sized JPEG, marked for every reader
    assert_eq!(named(&m, "cover").as_deref(), Some("cover-image"));
    assert!(opf.contains(
        "id=\"cover-image\" href=\"img/cover.jpg\" media-type=\"image/jpeg\" properties=\"cover-image\""
    ));
    assert!(opf.contains("<reference type=\"cover\""));
    assert!(
        e.text("OEBPS/nav.xhtml")
            .contains("epub:type=\"cover\" href=\"cover.xhtml\"")
    );
    let cover = &e.files["OEBPS/img/cover.jpg"];
    let img = image::load_from_memory(cover).unwrap();
    assert_eq!((img.width(), img.height()), (COVER_WIDTH, COVER_HEIGHT));
    assert!(cover.len() < 200 * 1024, "cover is {} bytes", cover.len());
    epubcheck("metadata.epub", &bytes);

    // the FB2 language wins over the catalog's; the FB2 series stays when the catalog has none
    let bytes = fb2_to_epub_with(
        &fixture("series1.fb2"),
        &opts,
        Assets::shared(),
        &BookMeta {
            lang: Some("ru".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let m = opf_metadata(&Epub::open(&bytes).text("OEBPS/content.opf"));
    assert_eq!(element(&m, "dc:language").as_deref(), Some("en"));
    assert_eq!(named(&m, "calibre:series_index").as_deref(), Some("1"));
}
