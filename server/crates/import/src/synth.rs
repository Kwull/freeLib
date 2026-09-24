//! Synthetic, Flibusta-like INPX generator (for tests and benchmarks).
//!
//! Output is deterministic for a given seed. Authors are drawn from a skewed distribution
//! over a pool of `books / 4` Russian-like names (shared surnames, patronymics), so some
//! authors have hundreds of books and most have a few. With `files_dir`, the matching zip
//! archives are written too, containing small valid FB2 files (every third one with a PNG cover).

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use freelib_catalog::genres::genres;
use freelib_catalog::util::{civil_from_days, days_from_civil};

/// Generator options.
#[derive(Debug, Clone)]
pub struct GenOptions {
    pub books: usize,
    /// Books per `.inp` part / zip archive.
    pub per_archive: usize,
    pub seed: u64,
    /// Also write zip archives with FB2 files into this folder.
    pub files_dir: Option<PathBuf>,
    /// Write an explicit `structure.info` (default order) into the INPX.
    pub structure_info: bool,
}

impl Default for GenOptions {
    fn default() -> Self {
        GenOptions { books: 1000, per_archive: 2000, seed: 42, files_dir: None, structure_info: true }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GenStats {
    pub books: usize,
    pub parts: usize,
    pub author_pool: usize,
    /// Total bytes of generated book files.
    pub file_bytes: u64,
}

/// SplitMix64: tiny, fast, good enough for synthetic data.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        let mut r = Rng(seed ^ 0x5DEE_CE66_D1CE_4E5B);
        r.next_u64();
        r
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
}

const SURNAMES: &[&str] = &[
    "Иванов", "Петров", "Сидоров", "Смирнов", "Кузнецов", "Попов", "Васильев", "Соколов", "Михайлов", "Новиков",
    "Фёдоров", "Морозов", "Волков", "Алексеев", "Лебедев", "Семёнов", "Егоров", "Павлов", "Козлов", "Степанов",
    "Николаев", "Орлов", "Андреев", "Макаров", "Никитин", "Захаров", "Зайцев", "Соловьёв", "Борисов", "Яковлев",
    "Григорьев", "Романов", "Воробьёв", "Сергеев", "Кузьмин", "Фролов", "Александров", "Дмитриев", "Королёв", "Гусев",
    "Киселёв", "Ильин", "Максимов", "Поляков", "Сорокин", "Виноградов", "Ковалёв", "Белов", "Медведев", "Антонов",
    "Тарасов", "Жуков", "Баранов", "Филиппов", "Комаров", "Давыдов", "Беляев", "Герасимов", "Богданов", "Осипов",
    "Сидорчук", "Панов", "Ершов", "Громов", "Тихонов", "Лукин", "Шестаков", "Стругацкий", "Лукьяненко", "Булычёв",
    "Беляков", "Муравьёв", "Шубин", "Ефремов", "Дьяченко", "Панкеев", "Злотников", "Круз", "Головачёв", "Перумов",
    "Никонов", "Абрамов", "Ярославцев", "Щербаков", "Юрьев", "Ёлкин", "Чернов", "Цветков", "Шмелёв", "Эйдельман",
];
const ROOTS: &[&str] = &[
    "Бел", "Черн", "Сер", "Рыж", "Кудр", "Лис", "Бобр", "Ворон", "Галк", "Грач", "Дятл", "Журавл", "Карп", "Щук",
    "Сом", "Окун", "Ерш", "Кот", "Пес", "Бык", "Козл", "Баран", "Конон", "Мельник", "Кузнец", "Гончар", "Плотник",
    "Столяр", "Пекар", "Рыбак", "Охотник", "Пахом", "Прох", "Тимоф", "Селиван", "Агафон", "Евдоким", "Ермол", "Зот",
    "Лаврент", "Мирон", "Нестер", "Остап", "Порфир", "Родион", "Савел", "Трофим", "Устин", "Фом", "Харитон", "Горб",
    "Долгорук", "Толст", "Тонк", "Лыс", "Глух", "Хромц", "Весел", "Добр", "Мудр", "Смел", "Тих", "Шум", "Гром", "Мороз",
    "Снеж", "Ветр", "Дожд", "Туман", "Рос", "Лес", "Бор", "Дуб", "Клён", "Берёз", "Осин", "Лип", "Ряб", "Калин",
];
const SUFFIXES: &[(&str, &str)] = &[("ов", "ова"), ("ин", "ина"), ("ский", "ская"), ("енко", "енко"), ("ович", "ович"), ("ев", "ева"), ("ицкий", "ицкая")];
const SYLLABLES: &[&str] = &[
    "ар", "ка", "мо", "ли", "тан", "гор", "эль", "ри", "до", "ва", "нор", "сет", "ам", "бер", "ви", "зан", "кир", "лон",
    "мар", "ос", "пер", "ру", "стан", "тор", "ул", "фен", "хал", "цен", "ша", "эр", "юн", "яр",
];
const MALE: &[&str] = &[
    "Александр", "Алексей", "Андрей", "Борис", "Вадим", "Василий", "Виктор", "Владимир", "Геннадий", "Георгий",
    "Дмитрий", "Евгений", "Иван", "Игорь", "Илья", "Кирилл", "Константин", "Леонид", "Максим", "Михаил",
    "Николай", "Олег", "Павел", "Пётр", "Роман", "Сергей", "Станислав", "Юрий", "Ярослав", "Фёдор",
];
const FEMALE: &[&str] = &[
    "Анна", "Алёна", "Валентина", "Вера", "Галина", "Дарья", "Екатерина", "Елена", "Ирина", "Ксения",
    "Людмила", "Мария", "Наталья", "Нина", "Ольга", "Светлана", "Татьяна", "Юлия", "Яна", "Марина",
];
const EN_LAST: &[&str] = &["Smith", "Brown", "Wilson", "Taylor", "Clarke", "Asimov", "Heinlein", "King", "O'Brien", "Pratchett"];
const EN_FIRST: &[&str] = &["John", "Robert", "Arthur", "Isaac", "Terry", "Mary", "Stephen", "Ursula", "Neil", "Anne"];
const ADJ: &[&str] = &[
    "Тёмный", "Последний", "Звёздный", "Красный", "Забытый", "Вечный", "Чёрный", "Белый", "Тайный", "Далёкий",
    "Холодный", "Огненный", "Мёртвый", "Живой", "Новый", "Старый", "Великий", "Маленький", "Золотой", "Серебряный",
    "Железный", "Стеклянный", "Горький", "Сладкий", "Северный", "Южный", "Лунный", "Солнечный", "Ночной", "Утренний",
];
const NOUN: &[&str] = &[
    "лес", "город", "мир", "путь", "берег", "рубеж", "дозор", "замок", "океан", "ветер", "меч", "щит", "огонь",
    "дом", "сад", "остров", "корабль", "камень", "король", "маг", "воин", "странник", "охотник", "след", "закат",
    "рассвет", "horizon", "космос", "портал", "легион", "архив", "код", "шторм", "лабиринт", "гамбит", "ключ",
];
const SERIES_PREFIX: &[&str] = &["Хроники", "Сага о", "Мир", "Легенды", "Приключения", "Дело", "Цикл", "Тайны", "Летопись", "Эпоха"];
const EN_WORDS: &[&str] = &["Dark", "Tower", "Last", "Star", "Road", "Night", "Empire", "Shadow", "River", "Dream", "Stone", "Fire"];
const KEYWORDS: &[&str] = &["магия", "космос", "попаданцы", "любовь", "детектив", "война", "драконы", "история", "будущее", "юмор"];
const LOREM: &[&str] = &[
    "Ветер гнал по небу рваные облака, и город внизу казался игрушечным.",
    "Он долго молчал, глядя на огонь, а потом сказал, что утром они уходят.",
    "В архиве пахло пылью и старой бумагой; где-то тикали часы.",
    "Никто не знал, откуда пришёл странник и куда он держит путь.",
    "Корабль вышел из гиперпространства точно в расчётной точке.",
    "Она улыбнулась и протянула ему ключ от старого дома у реки.",
];
const COVER_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAADwAAABaCAIAAABrM6JiAAAAZklEQVR42u3OQQkAMAgAQJPsvYhGNMlyLIWCcHABLvLcdUJaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaeiT9KteRlpaWlpaWlpaWlpaWlpaWlpaWlm7zAUmgjIi1osSpAAAAAElFTkSuQmCC";

struct Author {
    last: String,
    first: String,
    middle: String,
}

/// Patronymic from a father's first name (simplified Russian rules).
fn patronymic(father: &str, fem: bool) -> String {
    let base = match father {
        "Илья" => return if fem { "Ильинична".into() } else { "Ильич".into() },
        "Павел" => "Павл".to_string(),
        "Пётр" => "Петр".to_string(),
        "Фёдор" => "Федор".to_string(),
        "Василий" | "Юрий" | "Геннадий" | "Георгий" | "Евгений" | "Дмитрий" => {
            let stem = &father[..father.len() - "ий".len()];
            return format!("{stem}{}", if fem { "ьевна" } else { "ьевич" });
        }
        f if f.ends_with('й') => format!("{}е", &f[..f.len() - 'й'.len_utf8()]),
        f => f.to_string(),
    };
    if base.ends_with('е') {
        format!("{base}{}", if fem { "вна" } else { "вич" })
    } else {
        format!("{base}{}", if fem { "овна" } else { "ович" })
    }
}

fn author(seed: u64, i: usize) -> Author {
    let mut r = Rng::new(seed ^ (i as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
    if r.chance(0.05) {
        return Author { last: r.pick(EN_LAST).to_string(), first: r.pick(EN_FIRST).to_string(), middle: String::new() };
    }
    let female = r.chance(0.3);
    // Half the authors share common surnames (skewed), the rest get root + suffix surnames.
    let generated = r.chance(0.5);
    let (root, suffix) = (r.pick(ROOTS).to_string(), *r.pick(SUFFIXES));
    let si = ((SURNAMES.len() as f64) * r.unit().powf(1.6)) as usize;
    if generated {
        let last = format!("{root}{}", if female { suffix.1 } else { suffix.0 });
        let first = if female { r.pick(FEMALE) } else { r.pick(MALE) };
        let father: &str = MALE[r.below(MALE.len())];
        let middle = if r.chance(0.15) { String::new() } else { patronymic(father, female) };
        return Author { last, first: first.to_string(), middle };
    }
    let last = SURNAMES[si.min(SURNAMES.len() - 1)];
    let father = r.pick(MALE);
    let middle = if r.chance(0.15) { String::new() } else { patronymic(father, female) };
    if female {
        let last = if last.ends_with("ов") || last.ends_with("ев") || last.ends_with("ёв") || last.ends_with("ин") {
            format!("{last}а")
        } else if let Some(stem) = last.strip_suffix("ий") {
            format!("{stem}ая")
        } else {
            last.to_string()
        };
        Author { last, first: r.pick(FEMALE).to_string(), middle }
    } else {
        Author { last: last.to_string(), first: r.pick(MALE).to_string(), middle }
    }
}

fn series_name(seed: u64, sid: usize) -> String {
    let mut r = Rng::new(seed ^ 0xABCD ^ (sid as u64).wrapping_mul(0x9E37_79B9));
    let n = 2 + r.below(2);
    let proper: String = (0..n).map(|_| *r.pick(SYLLABLES)).collect();
    let mut chars = proper.chars();
    let proper: String = chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default();
    match r.below(4) {
        0 => format!("{} {}", r.pick(SERIES_PREFIX), proper),
        1 => format!("{} {}", r.pick(ADJ), r.pick(NOUN)),
        2 => format!("{} {} {}", r.pick(SERIES_PREFIX), r.pick(ADJ).to_lowercase(), r.pick(NOUN)),
        _ => format!("{proper}: {}", r.pick(NOUN)),
    }
}

fn title(r: &mut Rng, lang: &str) -> String {
    if lang == "en" {
        let n = 1 + r.below(3);
        return (0..n).map(|_| *r.pick(EN_WORDS)).collect::<Vec<_>>().join(" ");
    }
    let base = format!("{} {}", r.pick(ADJ), r.pick(NOUN));
    match r.below(10) {
        0 => format!("«{base}»"),
        1 => format!("{base}. Книга {}", 1 + r.below(5)),
        2 => r.pick(ADJ).to_string(),
        3 => format!("{base} и {}", r.pick(NOUN)),
        _ => base,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

struct GenBook {
    authors: Vec<Author>,
    genres: Vec<String>,
    title: String,
    series: String,
    serno: Option<usize>,
    lib_id: usize,
    deleted: bool,
    ext: &'static str,
    date: String,
    lang: &'static str,
    stars: usize,
    keywords: String,
    cover: bool,
    size: usize,
}

fn make_book(seed: u64, i: usize, n: usize, pool: usize, codes: &[String]) -> GenBook {
    let mut r = Rng::new(seed.wrapping_mul(31).wrapping_add(i as u64));
    let lib_id = 1000 + i;
    let lang: &'static str = match r.below(100) {
        0..=84 => "ru",
        85..=91 => "en",
        92..=95 => "uk",
        96 => "be",
        97 => "de",
        98 => "fr",
        _ => "pl",
    };
    let pick_author = |r: &mut Rng| ((pool as f64) * r.unit().powf(1.8)) as usize % pool;
    let mut authors = Vec::new();
    let a0 = pick_author(&mut r);
    if r.chance(0.003) {
        authors.push(Author { last: "Автор неизвестен".into(), first: String::new(), middle: String::new() });
    } else {
        authors.push(author(seed, a0));
        if r.chance(0.08) {
            let a1 = pick_author(&mut r);
            if a1 != a0 {
                authors.push(author(seed, a1));
            }
        }
    }
    // Each author owns up to 3 series; 40% of their books belong to one.
    let (series, serno) = if a0 % 4 != 0 && r.chance(0.4) {
        let sid = a0 * 3 + r.below(a0 % 4);
        (series_name(seed, sid), Some(1 + r.below(12)))
    } else {
        (String::new(), None)
    };
    let ng = match r.below(100) {
        0..=69 => 1,
        70..=94 => 2,
        _ => 3,
    };
    let mut genres: Vec<String> = Vec::new();
    for _ in 0..ng {
        let c = if r.chance(0.005) { "sf_brand_new".to_string() } else { r.pick(codes).clone() };
        if !genres.contains(&c) {
            genres.push(c);
        }
    }
    let ext = match r.below(100) {
        0..=96 => "fb2",
        97 => "epub",
        98 => "pdf",
        _ => "djvu",
    };
    // Dates grow with the library id (2007-01-01 .. 2026-09-01) plus a little noise.
    let d0 = days_from_civil(2007, 1, 1);
    let d1 = days_from_civil(2026, 9, 1);
    let day = d0 + ((d1 - d0) as f64 * i as f64 / n.max(1) as f64) as i64 + r.below(20) as i64;
    let (y, m, d) = civil_from_days(day.min(d1));
    GenBook {
        title: title(&mut r, lang),
        authors,
        genres,
        series,
        serno,
        lib_id,
        deleted: r.chance(0.08),
        ext,
        date: format!("{y:04}-{m:02}-{d:02}"),
        lang,
        stars: if r.chance(0.7) { 0 } else { 1 + r.below(5) },
        keywords: if r.chance(0.1) { format!("{}, {}", r.pick(KEYWORDS), r.pick(KEYWORDS)) } else { String::new() },
        cover: i % 3 == 0,
        size: 50_000 + r.below(2_000_000),
    }
}

/// A small but valid FB2 document for `b`.
fn fb2(b: &GenBook, cover_b64: &str) -> String {
    let mut s = String::with_capacity(2048);
    s.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<FictionBook xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\" xmlns:l=\"http://www.w3.org/1999/xlink\">\n<description><title-info>");
    for g in &b.genres {
        s.push_str(&format!("<genre>{}</genre>", esc(g)));
    }
    for a in &b.authors {
        s.push_str("<author>");
        if !a.first.is_empty() {
            s.push_str(&format!("<first-name>{}</first-name>", esc(&a.first)));
        }
        if !a.middle.is_empty() {
            s.push_str(&format!("<middle-name>{}</middle-name>", esc(&a.middle)));
        }
        s.push_str(&format!("<last-name>{}</last-name></author>", esc(&a.last)));
    }
    s.push_str(&format!("<book-title>{}</book-title>", esc(&b.title)));
    s.push_str(&format!(
        "<annotation><p>Синтетическая книга «{}» для проверки freeLib.</p><p>Вторая <emphasis>строка</emphasis> аннотации.</p></annotation>",
        esc(&b.title)
    ));
    if !b.keywords.is_empty() {
        s.push_str(&format!("<keywords>{}</keywords>", esc(&b.keywords)));
    }
    s.push_str(&format!("<date>{}</date>", &b.date[..4]));
    if b.cover {
        s.push_str("<coverpage><image l:href=\"#cover.png\"/></coverpage>");
    }
    s.push_str(&format!("<lang>{}</lang>", b.lang));
    if !b.series.is_empty() {
        s.push_str(&format!("<sequence name=\"{}\" number=\"{}\"/>", esc(&b.series), b.serno.unwrap_or(0)));
    }
    s.push_str(&format!(
        "</title-info><document-info><author><nickname>gen-inpx</nickname></author><date>{}</date><id>synthetic-{}</id><version>1.0</version></document-info></description>\n",
        b.date, b.lib_id
    ));
    s.push_str(&format!("<body><title><p>{}</p></title>\n", esc(&b.title)));
    for ch in 1..=2 {
        s.push_str(&format!("<section><title><p>Глава {ch}</p></title>"));
        for k in 0..3 {
            s.push_str(&format!("<p>{}</p>", LOREM[(b.lib_id + ch * 3 + k) % LOREM.len()]));
        }
        s.push_str("</section>\n");
    }
    s.push_str("</body>\n");
    if b.cover {
        s.push_str(&format!("<binary id=\"cover.png\" content-type=\"image/png\">{cover_b64}</binary>\n"));
    }
    s.push_str("</FictionBook>\n");
    s
}

fn inp_line(b: &GenBook, size: usize) -> String {
    let authors: String = b.authors.iter().map(|a| format!("{},{},{}:", a.last, a.first, a.middle)).collect();
    let genres: String = b.genres.iter().map(|g| format!("{g}:")).collect();
    let f = [
        authors,
        genres,
        b.title.clone(),
        b.series.clone(),
        b.serno.map(|n| n.to_string()).unwrap_or_default(),
        b.lib_id.to_string(),
        size.to_string(),
        b.lib_id.to_string(),
        if b.deleted { "1".into() } else { "0".into() },
        b.ext.to_string(),
        b.date.clone(),
        b.lang.to_string(),
        if b.stars > 0 { b.stars.to_string() } else { String::new() },
        b.keywords.clone(),
    ];
    let mut s = f.join("\x04");
    s.push_str("\x04\r\n");
    s
}

/// Generate `out` (an `.inpx`) and, optionally, the archives.
pub fn generate(out: &Path, opts: &GenOptions) -> io::Result<GenStats> {
    let n = opts.books;
    let per = opts.per_archive.max(1);
    let pool = (n / 4).max(20);
    let codes: Vec<String> = genres().all().iter().flat_map(|g| g.keys.iter().cloned()).collect();
    let parts = n.div_ceil(per);
    if let Some(dir) = &opts.files_dir {
        std::fs::create_dir_all(dir)?;
    }
    let results: Vec<io::Result<(String, String, u64)>> = (0..parts)
        .into_par_iter()
        .map(|p| {
            let lo = p * per;
            let hi = ((p + 1) * per).min(n);
            let name = format!("fb2-{:06}-{:06}", 1000 + lo, 1000 + hi - 1);
            let mut text = String::with_capacity((hi - lo) * 160);
            let mut bytes = 0u64;
            let mut zw = match &opts.files_dir {
                Some(dir) => Some(ZipWriter::new(BufWriter::new(File::create(dir.join(format!("{name}.zip")))?))),
                None => None,
            };
            let zopts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).compression_level(Some(1));
            for i in lo..hi {
                let b = make_book(opts.seed, i, n, pool, &codes);
                let size = if let Some(z) = zw.as_mut() {
                    let content: Vec<u8> = if b.ext == "fb2" {
                        fb2(&b, COVER_PNG_B64).into_bytes()
                    } else {
                        format!("synthetic {} file for book {}\n", b.ext, b.lib_id).into_bytes()
                    };
                    z.start_file(format!("{}.{}", b.lib_id, b.ext), zopts).map_err(io::Error::other)?;
                    z.write_all(&content)?;
                    bytes += content.len() as u64;
                    content.len()
                } else {
                    b.size
                };
                text.push_str(&inp_line(&b, size));
            }
            if let Some(z) = zw {
                z.finish().map_err(io::Error::other)?.flush()?;
            }
            Ok((format!("{name}.inp"), text, bytes))
        })
        .collect();

    let mut zw = ZipWriter::new(BufWriter::new(File::create(out)?));
    let zopts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zw.start_file("collection.info", zopts).map_err(io::Error::other)?;
    zw.write_all("Синтетическая библиотека (gen-inpx)\nsynthetic\n65536\nGenerated by freelib gen-inpx\n".as_bytes())?;
    zw.start_file("version.info", zopts).map_err(io::Error::other)?;
    zw.write_all(b"20260901\r\n")?;
    if opts.structure_info {
        zw.start_file("structure.info", zopts).map_err(io::Error::other)?;
        zw.write_all(crate::inpx::DEFAULT_STRUCTURE.as_bytes())?;
    }
    let mut stats = GenStats { books: n, parts, author_pool: pool, file_bytes: 0 };
    for r in results {
        let (name, text, bytes) = r?;
        zw.start_file(name, zopts).map_err(io::Error::other)?;
        zw.write_all(text.as_bytes())?;
        stats.file_bytes += bytes;
    }
    zw.finish().map_err(io::Error::other)?.flush()?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_look_russian() {
        let a = author(1, 5);
        assert!(!a.last.is_empty() && !a.first.is_empty());
        let mut female = 0;
        for i in 0..200 {
            let a = author(7, i);
            if a.middle.ends_with("вна") {
                female += 1;
                assert!(FEMALE.contains(&a.first.as_str()) || a.middle == "Ильинична", "{} {}", a.first, a.middle);
            }
        }
        assert!(female > 10);
    }

    #[test]
    fn deterministic() {
        let codes = vec!["sf".to_string()];
        let a = make_book(3, 10, 100, 25, &codes);
        let b = make_book(3, 10, 100, 25, &codes);
        assert_eq!(inp_line(&a, 1), inp_line(&b, 1));
    }
}
