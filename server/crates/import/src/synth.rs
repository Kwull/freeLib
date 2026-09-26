//! Synthetic, Flibusta-like INPX generator (for tests, benchmarks and UI review).
//!
//! Output is deterministic for a given seed. The author distribution is Zipf-like:
//!
//! * ~5% of the books belong to a fixed list of well-known authors (translated foreign
//!   authors in Cyrillic like "Азимов Айзек", "Шекли Роберт", Russian and Ukrainian classics,
//!   plus a few original-language Latin-script names with diacritics: "Lem Stanisław",
//!   "Čapek Karel", "Ødegaard Knut", "Əlibəyli Ramiz"…). The head of that list gets hundreds
//!   of books per 100k (Asimov ≈ 1000 per 100k books with anthologies), many series
//!   (including long publisher-series names), repeated famous titles, a regular co-author
//!   (the Strugatsky brothers) and an alternative spelling on a few books (near-duplicates);
//! * ~2.5% are anthologies/collections with 5–50 authors each, often including famous SF names;
//! * the rest are drawn from a skewed pool of `books / 2` Russian-like names (most with one
//!   or a few books), with Latin-script authors for most non-Russian books.
//!
//! Books have 1–6 genres, mostly `ru` with `uk`, `en`, `de`, `pl`, … and 8% are deleted;
//! about 2% have very long titles. With `files_dir`, the matching zip archives are written too,
//! containing small valid FB2 files (every third one with a PNG cover).

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use freelib_catalog::genres::genres;
use freelib_catalog::text::work_title_key_in_series;
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
    /// Add the fixed edition-detection showcase books (see `showcase_books`).
    pub showcase: bool,
}

impl Default for GenOptions {
    fn default() -> Self {
        GenOptions {
            books: 1000,
            per_archive: 2000,
            seed: 42,
            files_dir: None,
            structure_info: true,
            showcase: true,
        }
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
    "Иванов",
    "Петров",
    "Сидоров",
    "Смирнов",
    "Кузнецов",
    "Попов",
    "Васильев",
    "Соколов",
    "Михайлов",
    "Новиков",
    "Фёдоров",
    "Морозов",
    "Волков",
    "Алексеев",
    "Лебедев",
    "Семёнов",
    "Егоров",
    "Павлов",
    "Козлов",
    "Степанов",
    "Николаев",
    "Орлов",
    "Андреев",
    "Макаров",
    "Никитин",
    "Захаров",
    "Зайцев",
    "Соловьёв",
    "Борисов",
    "Яковлев",
    "Григорьев",
    "Романов",
    "Воробьёв",
    "Сергеев",
    "Кузьмин",
    "Фролов",
    "Александров",
    "Дмитриев",
    "Королёв",
    "Гусев",
    "Киселёв",
    "Ильин",
    "Максимов",
    "Поляков",
    "Сорокин",
    "Виноградов",
    "Ковалёв",
    "Белов",
    "Медведев",
    "Антонов",
    "Тарасов",
    "Жуков",
    "Баранов",
    "Филиппов",
    "Комаров",
    "Давыдов",
    "Беляев",
    "Герасимов",
    "Богданов",
    "Осипов",
    "Сидорчук",
    "Панов",
    "Ершов",
    "Громов",
    "Тихонов",
    "Лукин",
    "Шестаков",
    "Стругацкий",
    "Лукьяненко",
    "Булычёв",
    "Беляков",
    "Муравьёв",
    "Шубин",
    "Ефремов",
    "Дьяченко",
    "Панкеев",
    "Злотников",
    "Круз",
    "Головачёв",
    "Перумов",
    "Никонов",
    "Абрамов",
    "Ярославцев",
    "Щербаков",
    "Юрьев",
    "Ёлкин",
    "Чернов",
    "Цветков",
    "Шмелёв",
    "Эйдельман",
];
const ROOTS: &[&str] = &[
    "Бел",
    "Черн",
    "Сер",
    "Рыж",
    "Кудр",
    "Лис",
    "Бобр",
    "Ворон",
    "Галк",
    "Грач",
    "Дятл",
    "Журавл",
    "Карп",
    "Щук",
    "Сом",
    "Окун",
    "Ерш",
    "Кот",
    "Пес",
    "Бык",
    "Козл",
    "Баран",
    "Конон",
    "Мельник",
    "Кузнец",
    "Гончар",
    "Плотник",
    "Столяр",
    "Пекар",
    "Рыбак",
    "Охотник",
    "Пахом",
    "Прох",
    "Тимоф",
    "Селиван",
    "Агафон",
    "Евдоким",
    "Ермол",
    "Зот",
    "Лаврент",
    "Мирон",
    "Нестер",
    "Остап",
    "Порфир",
    "Родион",
    "Савел",
    "Трофим",
    "Устин",
    "Фом",
    "Харитон",
    "Горб",
    "Долгорук",
    "Толст",
    "Тонк",
    "Лыс",
    "Глух",
    "Хромц",
    "Весел",
    "Добр",
    "Мудр",
    "Смел",
    "Тих",
    "Шум",
    "Гром",
    "Мороз",
    "Снеж",
    "Ветр",
    "Дожд",
    "Туман",
    "Рос",
    "Лес",
    "Бор",
    "Дуб",
    "Клён",
    "Берёз",
    "Осин",
    "Лип",
    "Ряб",
    "Калин",
];
const SUFFIXES: &[(&str, &str)] = &[
    ("ов", "ова"),
    ("ин", "ина"),
    ("ский", "ская"),
    ("енко", "енко"),
    ("ович", "ович"),
    ("ев", "ева"),
    ("ицкий", "ицкая"),
];
const SYLLABLES: &[&str] = &[
    "ар", "ка", "мо", "ли", "тан", "гор", "эль", "ри", "до", "ва", "нор", "сет", "ам", "бер", "ви",
    "зан", "кир", "лон", "мар", "ос", "пер", "ру", "стан", "тор", "ул", "фен", "хал", "цен", "ша",
    "эр", "юн", "яр",
];
const MALE: &[&str] = &[
    "Александр",
    "Алексей",
    "Андрей",
    "Борис",
    "Вадим",
    "Василий",
    "Виктор",
    "Владимир",
    "Геннадий",
    "Георгий",
    "Дмитрий",
    "Евгений",
    "Иван",
    "Игорь",
    "Илья",
    "Кирилл",
    "Константин",
    "Леонид",
    "Максим",
    "Михаил",
    "Николай",
    "Олег",
    "Павел",
    "Пётр",
    "Роман",
    "Сергей",
    "Станислав",
    "Юрий",
    "Ярослав",
    "Фёдор",
];
const FEMALE: &[&str] = &[
    "Анна",
    "Алёна",
    "Валентина",
    "Вера",
    "Галина",
    "Дарья",
    "Екатерина",
    "Елена",
    "Ирина",
    "Ксения",
    "Людмила",
    "Мария",
    "Наталья",
    "Нина",
    "Ольга",
    "Светлана",
    "Татьяна",
    "Юлия",
    "Яна",
    "Марина",
];
const EN_LAST: &[&str] = &[
    "Smith",
    "Brown",
    "Wilson",
    "Taylor",
    "Clarke",
    "Asimov",
    "Heinlein",
    "King",
    "O'Brien",
    "Pratchett",
];
const EN_FIRST: &[&str] = &[
    "John", "Robert", "Arthur", "Isaac", "Terry", "Mary", "Stephen", "Ursula", "Neil", "Anne",
];
const ADJ: &[&str] = &[
    "Тёмный",
    "Последний",
    "Звёздный",
    "Красный",
    "Забытый",
    "Вечный",
    "Чёрный",
    "Белый",
    "Тайный",
    "Далёкий",
    "Холодный",
    "Огненный",
    "Мёртвый",
    "Живой",
    "Новый",
    "Старый",
    "Великий",
    "Маленький",
    "Золотой",
    "Серебряный",
    "Железный",
    "Стеклянный",
    "Горький",
    "Сладкий",
    "Северный",
    "Южный",
    "Лунный",
    "Солнечный",
    "Ночной",
    "Утренний",
];
const NOUN: &[&str] = &[
    "лес",
    "город",
    "мир",
    "путь",
    "берег",
    "рубеж",
    "дозор",
    "замок",
    "океан",
    "ветер",
    "меч",
    "щит",
    "огонь",
    "дом",
    "сад",
    "остров",
    "корабль",
    "камень",
    "король",
    "маг",
    "воин",
    "странник",
    "охотник",
    "след",
    "закат",
    "рассвет",
    "horizon",
    "космос",
    "портал",
    "легион",
    "архив",
    "код",
    "шторм",
    "лабиринт",
    "гамбит",
    "ключ",
];
const SERIES_PREFIX: &[&str] = &[
    "Хроники",
    "Сага о",
    "Мир",
    "Легенды",
    "Приключения",
    "Дело",
    "Цикл",
    "Тайны",
    "Летопись",
    "Эпоха",
];
const EN_WORDS: &[&str] = &[
    "Dark", "Tower", "Last", "Star", "Road", "Night", "Empire", "Shadow", "River", "Dream",
    "Stone", "Fire",
];
const KEYWORDS: &[&str] = &[
    "магия",
    "космос",
    "попаданцы",
    "любовь",
    "детектив",
    "война",
    "драконы",
    "история",
    "будущее",
    "юмор",
];
const LOREM: &[&str] = &[
    "Ветер гнал по небу рваные облака, и город внизу казался игрушечным.",
    "Он долго молчал, глядя на огонь, а потом сказал, что утром они уходят.",
    "В архиве пахло пылью и старой бумагой; где-то тикали часы.",
    "Никто не знал, откуда пришёл странник и куда он держит путь.",
    "Корабль вышел из гиперпространства точно в расчётной точке.",
    "Она улыбнулась и протянула ему ключ от старого дома у реки.",
];
const COVER_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAADwAAABaCAIAAABrM6JiAAAAZklEQVR42u3OQQkAMAgAQJPsvYhGNMlyLIWCcHABLvLcdUJaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaeiT9KteRlpaWlpaWlpaWlpaWlpaWlpaWlm7zAUmgjIi1osSpAAAAAElFTkSuQmCC";

#[derive(Clone)]
struct Author {
    last: String,
    first: String,
    middle: String,
}

/// Patronymic from a father's first name (simplified Russian rules).
fn patronymic(father: &str, fem: bool) -> String {
    let base = match father {
        "Илья" => {
            return if fem {
                "Ильинична".into()
            } else {
                "Ильич".into()
            };
        }
        "Павел" => "Павл".to_string(),
        "Пётр" => "Петр".to_string(),
        "Фёдор" => "Федор".to_string(),
        "Василий" | "Юрий" | "Геннадий" | "Георгий" | "Евгений" | "Дмитрий" =>
        {
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
        return Author {
            last: r.pick(EN_LAST).to_string(),
            first: r.pick(EN_FIRST).to_string(),
            middle: String::new(),
        };
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
        let middle = if r.chance(0.15) {
            String::new()
        } else {
            patronymic(father, female)
        };
        return Author {
            last,
            first: first.to_string(),
            middle,
        };
    }
    let last = SURNAMES[si.min(SURNAMES.len() - 1)];
    let father = r.pick(MALE);
    let middle = if r.chance(0.15) {
        String::new()
    } else {
        patronymic(father, female)
    };
    if female {
        let last = if last.ends_with("ов")
            || last.ends_with("ев")
            || last.ends_with("ёв")
            || last.ends_with("ин")
        {
            format!("{last}а")
        } else if let Some(stem) = last.strip_suffix("ий") {
            format!("{stem}ая")
        } else {
            last.to_string()
        };
        Author {
            last,
            first: r.pick(FEMALE).to_string(),
            middle,
        }
    } else {
        Author {
            last: last.to_string(),
            first: r.pick(MALE).to_string(),
            middle,
        }
    }
}

fn series_name(seed: u64, sid: usize) -> String {
    let mut r = Rng::new(seed ^ 0xABCD ^ (sid as u64).wrapping_mul(0x9E37_79B9));
    let n = 2 + r.below(2);
    let proper: String = (0..n).map(|_| *r.pick(SYLLABLES)).collect();
    let mut chars = proper.chars();
    let proper: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    match r.below(4) {
        0 => format!("{} {}", r.pick(SERIES_PREFIX), proper),
        1 => format!("{} {}", r.pick(ADJ), r.pick(NOUN)),
        2 => format!(
            "{} {} {}",
            r.pick(SERIES_PREFIX),
            r.pick(ADJ).to_lowercase(),
            r.pick(NOUN)
        ),
        _ => format!("{proper}: {}", r.pick(NOUN)),
    }
}

fn title(r: &mut Rng, lang: &str) -> String {
    if lang == "en" {
        let n = 1 + r.below(3);
        return (0..n)
            .map(|_| *r.pick(EN_WORDS))
            .collect::<Vec<_>>()
            .join(" ");
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
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[derive(Clone)]
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
    /// `<publish-info>`: publisher, year, ISBN as printed
    publish: Option<(&'static str, &'static str, &'static str)>,
}

/// A well-known author: most of the head of the Zipf-like author distribution
/// (a few names with hundreds of books each, like on Flibusta).
struct Famous {
    last: &'static str,
    first: &'static str,
    middle: &'static str,
    /// Relative share of the "famous" books.
    weight: u32,
    /// Language of their books (translations into Russian for foreign authors).
    lang: &'static str,
    genres: &'static [&'static str],
    titles: &'static [&'static str],
    series: &'static [&'static str],
    /// Another spelling of the same person, used on a few books (near-duplicate authors).
    alt: Option<(&'static str, &'static str, &'static str)>,
    /// Regular co-author (index into [`FAMOUS`]) and the share of books written together.
    partner: Option<(usize, f64)>,
    /// Appears in anthologies (science fiction, mostly).
    anthologies: bool,
}

const fn fam(
    last: &'static str,
    first: &'static str,
    middle: &'static str,
    weight: u32,
    lang: &'static str,
    genres: &'static [&'static str],
) -> Famous {
    Famous {
        last,
        first,
        middle,
        weight,
        lang,
        genres,
        titles: &[],
        series: &[],
        alt: None,
        partner: None,
        anthologies: false,
    }
}

const SF: &[&str] = &["sf", "sf_space", "sf_social"];
const DET: &[&str] = &["det_irony", "detective"];
const CLASSIC: &[&str] = &["prose_classic", "foreign_prose"];

/// Index of "Азимов Айзек" in [`FAMOUS`] (the prolific, anthology-heavy test case).
pub const ASIMOV: usize = 0;

static FAMOUS: &[Famous] = &[
    Famous {
        titles: &[
            "Я, робот",
            "Конец Вечности",
            "Основание",
            "Основание и Империя",
            "Второе Основание",
            "Сами боги",
            "Стальные пещеры",
            "Обнажённое солнце",
            "Роботы зари",
            "Двухсотлетний человек",
            "Приход ночи",
            "Немезида",
            "Камешек в небе",
            "Мечты роботов",
            "Путеводитель по Библии. Ветхий Завет",
            "Фантастическое путешествие",
        ],
        series: &[
            "Основание",
            "Роботы",
            "Галактическая Империя",
            "Лакки Старр",
            "Чёрные вдовы",
            "Азимов, Айзек. Сборники рассказов",
            "Норби",
            "Детективы Азимова",
        ],
        alt: Some(("Азимов", "Исаак", "")),
        partner: Some((36, 0.02)),
        anthologies: true,
        ..fam("Азимов", "Айзек", "", 200, "ru", SF)
    },
    Famous {
        series: &[
            "Любительница частного сыска Даша Васильева",
            "Евлампия Романова. Следствие ведёт дилетант",
            "Джентльмен сыска Иван Подушкин",
            "Виола Тараканова. В мире преступных страстей",
        ],
        ..fam("Донцова", "Дарья", "Аркадьевна", 150, "ru", DET)
    },
    fam(
        "Картленд",
        "Барбара",
        "",
        80,
        "ru",
        &["love_history", "love_contemporary"],
    ),
    Famous {
        titles: &[
            "Обмен разумов",
            "Цивилизация статуса",
            "Страж-птица",
            "Запах мысли",
            "Координаты чудес",
            "Билет на планету Транай",
        ],
        alt: Some(("Шекли", "Роберт", "Л.")),
        anthologies: true,
        ..fam("Шекли", "Роберт", "", 60, "ru", &["sf", "sf_humor"])
    },
    Famous {
        titles: &[
            "Оно",
            "Сияние",
            "Мизери",
            "Кэрри",
            "Под куполом",
            "Кладбище домашних животных",
        ],
        series: &["Тёмная Башня", "Билл Ходжес"],
        anthologies: true,
        ..fam("Кинг", "Стивен", "", 70, "ru", &["sf_horror", "thriller"])
    },
    Famous {
        titles: &[
            "451° по Фаренгейту",
            "Марсианские хроники",
            "Вино из одуванчиков",
            "И грянул гром",
        ],
        anthologies: true,
        ..fam("Брэдбери", "Рэй", "Дуглас", 50, "ru", SF)
    },
    Famous {
        titles: &[
            "Космическая одиссея 2001 года",
            "Свидание с Рамой",
            "Конец детства",
        ],
        alt: Some(("Кларк", "Артур", "")),
        anthologies: true,
        ..fam("Кларк", "Артур", "Чарльз", 45, "ru", SF)
    },
    Famous {
        titles: &[
            "Звёздный десант",
            "Дверь в лето",
            "Чужак в стране чужой",
            "Кукловоды",
        ],
        anthologies: true,
        ..fam("Хайнлайн", "Роберт", "Энсон", 45, "ru", SF)
    },
    Famous {
        titles: &["Город", "Заповедник гоблинов", "Пересадочная станция"],
        anthologies: true,
        ..fam("Саймак", "Клиффорд", "Дональд", 40, "ru", SF)
    },
    Famous {
        series: &["Хроники Амбера"],
        anthologies: true,
        ..fam(
            "Желязны",
            "Роджер",
            "Джозеф",
            40,
            "ru",
            &["sf_fantasy", "sf"],
        )
    },
    Famous {
        titles: &[
            "Солярис",
            "Непобедимый",
            "Звёздные дневники Ийона Тихого",
            "Кибериада",
        ],
        anthologies: true,
        ..fam("Лем", "Станислав", "", 35, "ru", SF)
    },
    Famous {
        titles: &[
            "Пикник на обочине",
            "Трудно быть богом",
            "Понедельник начинается в субботу",
            "Улитка на склоне",
            "Град обреченный",
        ],
        series: &["Мир Полудня", "НИИЧАВО"],
        partner: Some((12, 0.85)),
        anthologies: true,
        ..fam("Стругацкий", "Аркадий", "Натанович", 50, "ru", SF)
    },
    Famous {
        partner: Some((11, 0.6)),
        ..fam("Стругацкий", "Борис", "Натанович", 15, "ru", SF)
    },
    Famous {
        titles: &[
            "Ночной дозор",
            "Дневной дозор",
            "Спектр",
            "Черновик",
            "Лабиринт отражений",
        ],
        series: &["Дозоры", "Лабиринт отражений", "Черновик"],
        anthologies: true,
        ..fam(
            "Лукьяненко",
            "Сергей",
            "Васильевич",
            60,
            "ru",
            &["sf", "sf_action"],
        )
    },
    Famous {
        series: &[
            "Приключения Алисы",
            "Великий Гусляр",
            "Интергалактическая полиция (ИнтерГпол)",
        ],
        anthologies: true,
        ..fam("Булычёв", "Кир", "", 55, "ru", &["sf", "child_sf"])
    },
    Famous {
        series: &["Плоский мир", "Плоский мир: Стража", "Плоский мир: Ведьмы"],
        ..fam(
            "Пратчетт",
            "Терри",
            "",
            45,
            "ru",
            &["sf_humor", "sf_fantasy"],
        )
    },
    Famous {
        series: &["Шерлок Холмс", "Профессор Челленджер"],
        anthologies: true,
        ..fam(
            "Дойл",
            "Артур",
            "Конан",
            40,
            "ru",
            &["det_classic", "adv_history"],
        )
    },
    Famous {
        series: &["Эркюль Пуаро", "Мисс Марпл", "Томми и Таппенс"],
        ..fam("Кристи", "Агата", "", 60, "ru", &["det_classic"])
    },
    Famous {
        series: &["Комиссар Мегрэ"],
        ..fam(
            "Сименон",
            "Жорж",
            "",
            50,
            "ru",
            &["det_classic", "det_police"],
        )
    },
    Famous {
        titles: &["Три мушкетёра", "Граф Монте-Кристо", "Королева Марго"],
        series: &["Д'Артаньян", "Валуа"],
        ..fam(
            "Дюма",
            "Александр",
            "",
            45,
            "ru",
            &["adv_history", "prose_classic"],
        )
    },
    fam("Верн", "Жюль", "", 40, "ru", &["adventure", "sf"]),
    fam(
        "Лондон",
        "Джек",
        "",
        35,
        "ru",
        &["adventure", "foreign_prose"],
    ),
    fam("Чехов", "Антон", "Павлович", 50, "ru", CLASSIC),
    fam("Толстой", "Лев", "Николаевич", 40, "ru", CLASSIC),
    Famous {
        series: &["Стальная Крыса", "Мир смерти", "Билл — герой Галактики"],
        anthologies: true,
        ..fam("Гаррисон", "Гарри", "", 40, "ru", &["sf", "sf_humor"])
    },
    Famous {
        anthologies: true,
        ..fam("Андерсон", "Пол", "Уильям", 35, "ru", SF)
    },
    Famous {
        series: &["Земноморье", "Хайнский цикл"],
        anthologies: true,
        ..fam(
            "Ле Гуин",
            "Урсула",
            "Кребер",
            30,
            "ru",
            &["sf_fantasy", "sf"],
        )
    },
    Famous {
        series: &["Сварог", "Бешеная", "Пиранья"],
        ..fam(
            "Бушков",
            "Александр",
            "Александрович",
            45,
            "ru",
            &["sf_action", "det_action"],
        )
    },
    Famous {
        series: &["Анастасия Каменская"],
        ..fam(
            "Маринина",
            "Александра",
            "Борисовна",
            35,
            "ru",
            &["det_police"],
        )
    },
    fam(
        "Головачёв",
        "Василий",
        "Васильевич",
        35,
        "ru",
        &["sf", "sf_action"],
    ),
    fam("Перумов", "Ник", "", 35, "ru", &["sf_fantasy"]),
    fam(
        "Злотников",
        "Роман",
        "Валерьевич",
        30,
        "ru",
        &["sf_action", "sf_history"],
    ),
    Famous {
        anthologies: true,
        ..fam("Нортон", "Андрэ", "", 30, "ru", &["sf", "sf_fantasy"])
    },
    Famous {
        anthologies: true,
        ..fam("Гамильтон", "Эдмонд", "Мур", 25, "ru", SF)
    },
    fam(
        "Шилова",
        "Юлия",
        "Витальевна",
        30,
        "ru",
        &["love_contemporary"],
    ),
    fam(
        "Шевченко",
        "Тарас",
        "Григорович",
        10,
        "uk",
        &["poetry", "prose_classic"],
    ),
    Famous {
        partner: Some((ASIMOV, 0.1)),
        anthologies: true,
        ..fam("Силверберг", "Роберт", "", 25, "ru", SF)
    },
    fam("Франко", "Іван", "Якович", 10, "uk", &["prose_classic"]),
    fam("Єрмоленко", "Ґанна", "", 3, "uk", &["prose_contemporary"]),
    // Original-language editions (Latin script, diacritics).
    Famous {
        titles: &["Solaris", "Niezwyciężony", "Cyberiada"],
        ..fam("Lem", "Stanisław", "", 8, "pl", SF)
    },
    Famous {
        titles: &["Válka s mloky", "R.U.R.", "Krakatit"],
        ..fam("Čapek", "Karel", "", 8, "cs", &["sf", "dramaturgy"])
    },
    Famous {
        titles: &[
            "I, Robot",
            "Foundation",
            "The End of Eternity",
            "The Caves of Steel",
        ],
        anthologies: true,
        ..fam("Asimov", "Isaac", "", 10, "en", SF)
    },
    fam("Ødegaard", "Knut", "", 3, "nb", &["prose_contemporary"]),
    fam("Hašek", "Jaroslav", "", 4, "cs", &["humor_prose"]),
    fam("Sienkiewicz", "Henryk", "", 4, "pl", &["adv_history"]),
    fam(
        "García Márquez",
        "Gabriel",
        "",
        4,
        "es",
        &["prose_contemporary"],
    ),
    fam("Böll", "Heinrich", "", 4, "de", &["prose_contemporary"]),
    fam("Əlibəyli", "Ramiz", "", 2, "az", &["poetry"]),
    fam("Ðukić", "Dragana", "", 2, "sr", &["prose_contemporary"]),
    fam("Þórðarson", "Þórbergur", "", 2, "is", &["prose_classic"]),
    fam("Öztürk", "Ayşe", "", 2, "tr", &["love_contemporary"]),
    fam("Larsson", "Åsa", "", 3, "sv", &["det_police"]),
    fam("Żeromski", "Stefan", "", 2, "pl", &["prose_classic"]),
    fam("Šalamun", "Tomaž", "", 2, "sl", &["poetry"]),
    fam("Ølstad", "Lars", "", 2, "nb", &["prose_contemporary"]),
    fam("Éluard", "Paul", "", 2, "fr", &["poetry"]),
];

/// Share of books written by [`FAMOUS`] authors (as their own books, not anthologies).
const FAMOUS_SHARE: f64 = 0.05;
/// Share of anthologies / collections (5–50 authors each).
const ANTHOLOGY_SHARE: f64 = 0.025;

const LATIN_LAST: &[&str] = &[
    "Müller",
    "Schäfer",
    "Dvořák",
    "Novák",
    "Kowalski",
    "Wiśniewski",
    "Nowak",
    "Łukasiewicz",
    "Østergaard",
    "Åberg",
    "Ångström",
    "Johansson",
    "Nielsen",
    "Jørgensen",
    "Ferreira",
    "Gonçalves",
    "Muñoz",
    "Peña",
    "Dubois",
    "Lefèvre",
    "Mercier",
    "Rossi",
    "Bianchi",
    "Çelik",
    "Yılmaz",
    "Horváth",
    "Kovačević",
    "Šimić",
    "Brontë",
    "O'Connor",
    "Smith",
    "Walker",
    "Hughes",
    "Weiß",
];
const LATIN_FIRST: &[&str] = &[
    "Jan",
    "Jiří",
    "Łukasz",
    "Zoë",
    "Søren",
    "José",
    "Émile",
    "François",
    "Małgorzata",
    "Björn",
    "Ülle",
    "İlker",
    "Anna",
    "Thomas",
    "Maria",
    "Peter",
    "Agnieszka",
    "Luís",
    "Ingrid",
    "Mark",
];

const ANTHOLOGY_SERIES: &[&str] = &[
    "Антология фантастики",
    "Библиотека современной фантастики",
    "Антология «Лучшее за год»",
    "Шедевры фантастики (продолжатели)",
    "Зарубежная фантастика (изд-во «Мир»)",
    "Антология мировой фантастики (Аванта+)",
    "Библиотека приключений и научной фантастики (Детская литература, 1955–1995)",
    "Сборники рассказов журнала «Если»",
    "Мастера детектива: антология классического детектива в двадцати томах с комментариями",
];
const PUBLISHER_SERIES: &[&str] = &[
    "Зарубежная фантастика (изд-во «Мир»)",
    "Классика мировой фантастики (Азбука)",
    "Эксклюзивная классика (АСТ)",
    "Библиотека всемирной литературы",
    "Мастера фантастики (Эксмо)",
    "Золотой фонд мировой классики",
    "Звёзды мировой фантастики",
    "Фантастика: классика и современность — большая серия избранных произведений в твёрдом переплёте",
    "Собрание сочинений в пятнадцати томах",
];
const ANTHOLOGY_TITLE: &[&str] = &[
    "Антология фантастики",
    "Лучшее за год: Мир фантастики",
    "Шедевры фантастики",
    "Зарубежная фантастика. Сборник рассказов",
    "Роботы и люди",
    "Иностранная фантастика. Выпуск",
    "Библиотека современной фантастики. Том",
    "Новая космическая опера",
    "Антология мировой фантастики. Том",
    "Фантастика",
    "Звёздный путь: антология",
    "Детективы Ellery Queen's Mystery Magazine",
];

/// Cumulative weights of [`FAMOUS`].
fn famous_cumulative() -> &'static [u32] {
    static CUM: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    CUM.get_or_init(|| {
        let mut acc = 0;
        FAMOUS
            .iter()
            .map(|f| {
                acc += f.weight;
                acc
            })
            .collect()
    })
}

fn pick_famous(r: &mut Rng, anthologies_only: bool) -> usize {
    loop {
        let cum = famous_cumulative();
        let x = r.below(*cum.last().unwrap() as usize) as u32;
        let k = cum.partition_point(|&c| c <= x);
        if !anthologies_only || FAMOUS[k].anthologies {
            return k;
        }
    }
}

fn famous_author(k: usize) -> Author {
    let f = &FAMOUS[k];
    Author {
        last: f.last.into(),
        first: f.first.into(),
        middle: f.middle.into(),
    }
}

/// A Latin-script author (original-language books): mostly one or two books each.
fn latin_author(seed: u64, i: usize) -> Author {
    let mut r = Rng::new(seed ^ 0x1A71 ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    Author {
        last: r.pick(LATIN_LAST).to_string(),
        first: r.pick(LATIN_FIRST).to_string(),
        middle: String::new(),
    }
}

/// A very long title (Flibusta has plenty of 150+ character ones).
fn long_title(r: &mut Rng) -> String {
    format!(
        "{} {}, или Подлинная история о том, как {} {} искал {} и что из этого вышло: роман в трёх частях с прологом, эпилогом и примечаниями составителя",
        r.pick(ADJ),
        r.pick(NOUN),
        r.pick(ADJ).to_lowercase(),
        r.pick(NOUN),
        r.pick(NOUN)
    )
}

fn book_genres(r: &mut Rng, codes: &[String], preferred: &[&str]) -> Vec<String> {
    // Most books have one or two genres, a few have up to six.
    let ng = match r.below(100) {
        0..=54 => 1,
        55..=79 => 2,
        80..=91 => 3,
        _ => 4 + r.below(3),
    };
    let mut genres: Vec<String> = Vec::new();
    for k in 0..ng {
        let c = if k == 0 && !preferred.is_empty() {
            r.pick(preferred).to_string()
        } else if r.chance(0.005) {
            "sf_brand_new".to_string()
        } else {
            r.pick(codes).clone()
        };
        if !genres.contains(&c) {
            genres.push(c);
        }
    }
    genres
}

fn pick_lang(r: &mut Rng) -> &'static str {
    match r.below(100) {
        0..=79 => "ru",
        80..=85 => "en",
        86..=90 => "uk",
        91..=92 => "de",
        93..=94 => "pl",
        95 => "be",
        96 => "fr",
        97 => "cs",
        98 => "es",
        _ => "it",
    }
}

fn make_book(seed: u64, i: usize, n: usize, pool: usize, codes: &[String]) -> GenBook {
    let mut r = Rng::new(seed.wrapping_mul(31).wrapping_add(i as u64));
    let lib_id = 1000 + i;
    let pick_author = |r: &mut Rng| ((pool as f64) * r.unit().powf(2.2)) as usize % pool;
    let kind = r.unit();
    let mut authors = Vec::new();
    let mut series = String::new();
    let mut serno = None;
    // an author's own series (not a publisher's or an anthology series): numbered by title
    let mut own_series = false;
    let lang: &'static str;
    let mut title_s: Option<String> = None;
    let preferred: &[&str] = if kind < ANTHOLOGY_SHARE {
        // Anthology / collection: 5–50 authors (mostly 5–15), famous SF names among them.
        lang = if r.chance(0.9) { "ru" } else { "en" };
        let count = 5 + (45.0 * r.unit().powf(2.5)) as usize;
        let mut seen = std::collections::HashSet::new();
        while authors.len() < count {
            let a = if r.chance(0.06) {
                let k = pick_famous(&mut r, true);
                if !seen.insert(("f", k)) {
                    continue;
                }
                famous_author(k)
            } else {
                let k = pick_author(&mut r);
                if !seen.insert(("p", k)) {
                    continue;
                }
                author(seed, k)
            };
            authors.push(a);
        }
        let t = r.pick(ANTHOLOGY_TITLE);
        title_s = Some(if t.ends_with("Выпуск") || t.ends_with("Том") {
            format!("{t} {}", 1 + r.below(40))
        } else if r.chance(0.5) {
            format!("{t}-{}", 1960 + r.below(60))
        } else {
            t.to_string()
        });
        if r.chance(0.6) {
            series = r.pick(ANTHOLOGY_SERIES).to_string();
            serno = Some(1 + r.below(60));
        }
        &["sf", "sf_social", "det_classic"]
    } else if kind < ANTHOLOGY_SHARE + FAMOUS_SHARE {
        let k = pick_famous(&mut r, false);
        let f = &FAMOUS[k];
        lang = if f.lang == "ru" && r.chance(0.04) {
            "uk"
        } else {
            f.lang
        };
        match f.alt {
            Some((l, fi, m)) if r.chance(0.04) => authors.push(Author {
                last: l.into(),
                first: fi.into(),
                middle: m.into(),
            }),
            _ => authors.push(famous_author(k)),
        }
        if let Some((p, share)) = f.partner
            && r.chance(share)
        {
            authors.push(famous_author(p));
        }
        if !f.titles.is_empty() && r.chance(0.35) {
            // Several editions of the same famous title; some marked as another translation
            // (decided by the index, so the random stream stays as it was).
            let t = r.pick(f.titles);
            title_s = Some(match i.wrapping_mul(2_654_435_761) % 23 {
                0 => format!("{t} (другой перевод)"),
                1 => format!("{t} [иллюстрации]"),
                _ => t.to_string(),
            });
        }
        // Own series: the listed ones first, then generated ones, roughly one per 6 books;
        // some books are in (long-named) publisher series instead.
        let expected = (n as f64 * FAMOUS_SHARE * f.weight as f64
            / *famous_cumulative().last().unwrap() as f64) as usize;
        let own = (expected / 6).clamp(f.series.len().max(1), 150);
        if r.chance(0.65) {
            if r.chance(0.2) {
                series = r.pick(PUBLISHER_SERIES).to_string();
            } else {
                let j = ((own as f64) * r.unit().powf(1.5)) as usize;
                series = match f.series.get(j) {
                    Some(s) => s.to_string(),
                    None => series_name(seed, 10_000_000 + k * 1000 + j),
                };
                own_series = true;
            }
            serno = Some(1 + r.below(15));
        }
        f.genres
    } else {
        lang = pick_lang(&mut r);
        let a0 = pick_author(&mut r);
        if r.chance(0.003) {
            authors.push(Author {
                last: "Автор неизвестен".into(),
                first: String::new(),
                middle: String::new(),
            });
        } else if lang != "ru" && lang != "uk" && lang != "be" && r.chance(0.6) {
            authors.push(latin_author(seed, a0));
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
        if a0 % 4 != 0 && r.chance(0.4) {
            let sid = a0 * 3 + r.below(a0 % 4);
            series = series_name(seed, sid);
            serno = Some(1 + r.below(12));
            own_series = true;
        }
        &[]
    };
    let genres = book_genres(&mut r, codes, preferred);
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
    let title = match title_s {
        Some(t) => t,
        None if r.chance(0.02) => long_title(&mut r),
        None => title(&mut r, lang),
    };
    if own_series {
        // One number per title (editions of a title share it, other titles rarely do), so that
        // the importer's series-number rule joins editions, not unrelated random titles.
        let key = work_title_key_in_series(&title, None).unwrap_or_default();
        let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
        });
        serno = Some(1 + (h % 97) as usize);
    }
    GenBook {
        title,
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
        keywords: if r.chance(0.1) {
            format!("{}, {}", r.pick(KEYWORDS), r.pick(KEYWORDS))
        } else {
            String::new()
        },
        cover: i % 3 == 0,
        size: 50_000 + r.below(2_000_000),
        publish: match i % 5 {
            0 => Some(("Эксмо", "2012", "978-5-699-12014-7")),
            1 => Some(("АСТ", "2008", "5-17-012345-0; 978-5-17-012345-2")),
            _ => None,
        },
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
        s.push_str(&format!(
            "<sequence name=\"{}\" number=\"{}\"/>",
            esc(&b.series),
            b.serno.unwrap_or(0)
        ));
    }
    s.push_str("</title-info>");
    if let Some((publisher, year, isbn)) = b.publish {
        s.push_str(&format!(
            "<publish-info><book-name>{}</book-name><publisher>{}</publisher><year>{year}</year><isbn>{}</isbn></publish-info>",
            esc(&b.title),
            esc(publisher),
            esc(isbn)
        ));
    }
    s.push_str(&format!(
        "<document-info><author><nickname>gen-inpx</nickname></author><date>{}</date><id>synthetic-{}</id><version>1.0</version></document-info></description>\n",
        b.date, b.lib_id
    ));
    s.push_str(&format!("<body><title><p>{}</p></title>\n", esc(&b.title)));
    for ch in 1..=2 {
        s.push_str(&format!("<section><title><p>Глава {ch}</p></title>"));
        for k in 0..3 {
            s.push_str(&format!(
                "<p>{}</p>",
                LOREM[(b.lib_id + ch * 3 + k) % LOREM.len()]
            ));
        }
        s.push_str("</section>\n");
    }
    s.push_str("</body>\n");
    if b.cover {
        s.push_str(&format!(
            "<binary id=\"cover.png\" content-type=\"image/png\">{cover_b64}</binary>\n"
        ));
    }
    s.push_str("</FictionBook>\n");
    s
}

fn inp_line(b: &GenBook, size: usize) -> String {
    let authors: String = b
        .authors
        .iter()
        .map(|a| format!("{},{},{}:", a.last, a.first, a.middle))
        .collect();
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
        if b.stars > 0 {
            b.stars.to_string()
        } else {
            String::new()
        },
        b.keywords.clone(),
    ];
    let mut s = f.join("\x04");
    s.push_str("\x04\r\n");
    s
}

/// Fixed books that show how editions are detected (lib ids 100..): Asimov's Foundation
/// novels in several Russian translations under different titles and numbers of the series
/// «Академия [Азимов]» (as in a real Flibusta library), unnumbered omnibus volumes, and
/// Marinina's two-volume «Люди за спиной» (Каменская #37).
fn showcase_books() -> Vec<GenBook> {
    let asimov = || vec![famous_author(ASIMOV)];
    let marinina = || {
        vec![Author {
            last: "Маринина".into(),
            first: "Александра".into(),
            middle: "Борисовна".into(),
        }]
    };
    const A: &str = "Академия [Азимов]";
    const K: &str = "Каменская";
    let rows: Vec<(&str, &str, Option<usize>, bool)> = vec![
        ("Прелюдия к Академии", A, Some(1), true),
        ("Прелюдия к Основанию", A, Some(1), true),
        ("Миры Айзека Азимова. Книга 5", A, Some(1), true),
        ("Академия", A, Some(3), true),
        ("Основание", A, Some(3), true),
        ("Основание (другой перевод)", A, Some(3), true),
        ("Установление", A, Some(3), true),
        ("Фонд", A, Some(2), true),
        ("Фонд [litres]", A, Some(2), true),
        ("Второй Фонд", A, Some(5), true),
        ("Дублеры", A, Some(5), true),
        ("Академия на краю гибели", A, Some(6), true),
        ("Академия на краю гибели", A, Some(6), true),
        ("Академия на краю гибели (fb2)", A, Some(6), true),
        ("Край Основания", A, Some(6), true),
        ("Миры Айзека Азимова. Книга 9", A, Some(6), true),
        ("Сообщество на краю", A, Some(6), true),
        ("Академия и Земля", A, Some(7), true),
        ("Академия и Земля", A, Some(7), true),
        ("Миры Айзека Азимова. Книга 10", A, Some(7), true),
        ("Основание и Земля", A, Some(7), true),
        ("Сообщество и Земля", A, Some(7), true),
        ("Страхи Академии", A, Some(8), true),
        ("Академия и Хаос", A, Some(9), true),
        ("Триумф Академии", A, Some(10), true),
        ("Академия. Книги 1-7", A, None, true),
        ("Академия. Начало", A, None, true),
        ("Академия. Первая трилогия", A, None, true),
        ("Миры Айзека Азимова. Книга 7", A, None, true),
        ("Путь к Академии", A, None, true),
        ("Люди за спиной. Том 1", K, Some(37), false),
        ("Люди за спиной, том 1", K, Some(37), false),
        ("Люди за спиной. Том 2", K, Some(37), false),
    ];
    rows.into_iter()
        .enumerate()
        .map(|(j, (title, series, serno, is_asimov))| GenBook {
            authors: if is_asimov { asimov() } else { marinina() },
            genres: vec![if is_asimov { "sf" } else { "det_police" }.to_string()],
            title: title.to_string(),
            series: series.to_string(),
            serno,
            lib_id: 100 + j,
            deleted: false,
            ext: "fb2",
            date: format!("20{:02}-0{}-1{}", 8 + j % 10, 1 + j % 9, j % 10),
            lang: "ru",
            stars: if j % 3 == 0 { 4 } else { 0 },
            keywords: String::new(),
            cover: j % 2 == 0,
            size: 300_000 + j * 17_000,
            publish: match j % 3 {
                0 => Some(("Эксмо", "2018", "978-5-699-12014-7")),
                1 => Some(("АСТ", "2012", "ISBN 5-17-012345-0")),
                _ => None,
            },
        })
        .collect()
}

/// Generate `out` (an `.inpx`) and, optionally, the archives.
pub fn generate(out: &Path, opts: &GenOptions) -> io::Result<GenStats> {
    let n = opts.books;
    let per = opts.per_archive.max(1);
    let pool = (n / 2).max(20);
    let codes: Vec<String> = genres()
        .all()
        .iter()
        .flat_map(|g| g.keys.iter().cloned())
        .collect();
    let parts = n.div_ceil(per);
    // the last part also carries the showcase books
    let showcase = if opts.showcase {
        showcase_books()
    } else {
        Vec::new()
    };
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
                Some(dir) => Some(ZipWriter::new(BufWriter::new(File::create(
                    dir.join(format!("{name}.zip")),
                )?))),
                None => None,
            };
            let zopts = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(1));
            let extra: &[GenBook] = if p + 1 == parts { &showcase } else { &[] };
            let made = (lo..hi).map(|i| make_book(opts.seed, i, n, pool, &codes));
            for b in made.chain(extra.iter().cloned()) {
                let size = if let Some(z) = zw.as_mut() {
                    let content: Vec<u8> = if b.ext == "fb2" {
                        fb2(&b, COVER_PNG_B64).into_bytes()
                    } else {
                        format!("synthetic {} file for book {}\n", b.ext, b.lib_id).into_bytes()
                    };
                    z.start_file(format!("{}.{}", b.lib_id, b.ext), zopts)
                        .map_err(io::Error::other)?;
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
    zw.start_file("collection.info", zopts)
        .map_err(io::Error::other)?;
    zw.write_all(
        "Синтетическая библиотека (gen-inpx)\nsynthetic\n65536\nGenerated by freelib gen-inpx\n"
            .as_bytes(),
    )?;
    zw.start_file("version.info", zopts)
        .map_err(io::Error::other)?;
    zw.write_all(b"20260901\r\n")?;
    if opts.structure_info {
        zw.start_file("structure.info", zopts)
            .map_err(io::Error::other)?;
        zw.write_all(crate::inpx::DEFAULT_STRUCTURE.as_bytes())?;
    }
    let mut stats = GenStats {
        books: n + showcase.len(),
        parts,
        author_pool: pool,
        file_bytes: 0,
    };
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
                assert!(
                    FEMALE.contains(&a.first.as_str()) || a.middle == "Ильинична",
                    "{} {}",
                    a.first,
                    a.middle
                );
            }
        }
        assert!(female > 10);
    }

    #[test]
    fn famous_table_indices() {
        assert_eq!(FAMOUS[ASIMOV].last, "Азимов");
        for (k, f) in FAMOUS.iter().enumerate() {
            if let Some((p, _)) = f.partner {
                assert!(p < FAMOUS.len() && p != k, "{}", f.last);
            }
        }
        assert_eq!(FAMOUS[36].last, "Силверберг");
        assert_eq!(FAMOUS[11].partner.unwrap().0, 12);
    }

    #[test]
    fn zipf_like_distribution_with_anthologies() {
        let codes: Vec<String> = genres()
            .all()
            .iter()
            .flat_map(|g| g.keys.iter().cloned())
            .collect();
        let n = 20_000;
        let mut per_author: std::collections::HashMap<String, usize> = Default::default();
        let (mut anthologies, mut latin, mut long) = (0, 0, 0);
        for i in 0..n {
            let b = make_book(42, i, n, n / 2, &codes);
            if b.authors.len() >= 5 {
                anthologies += 1;
            }
            if b.title.chars().count() > 120 {
                long += 1;
            }
            for a in &b.authors {
                if a.last.chars().any(|c| c.is_ascii_alphabetic()) {
                    latin += 1;
                }
                *per_author
                    .entry(format!("{} {} {}", a.last, a.first, a.middle))
                    .or_default() += 1;
            }
        }
        let asimov = per_author["Азимов Айзек "];
        assert!((120..400).contains(&asimov), "asimov: {asimov}");
        let singles = per_author.values().filter(|&&c| c == 1).count();
        assert!(
            singles > per_author.len() / 4,
            "{singles} of {}",
            per_author.len()
        );
        assert!(anthologies > 300 && latin > 300 && long > 100);
        assert!(per_author.contains_key("Čapek Karel "));
    }

    #[test]
    fn deterministic() {
        let codes = vec!["sf".to_string()];
        let a = make_book(3, 10, 100, 25, &codes);
        let b = make_book(3, 10, 100, 25, &codes);
        assert_eq!(inp_line(&a, 1), inp_line(&b, 1));
    }
}
