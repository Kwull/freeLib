//! Embedded resources: fonts, CSS and hyphenation dictionaries.
//! Copied from the Qt app (`freeLib/src/xsl`) into `assets/` so the crate builds without the
//! Qt tree (the Docker context excludes it). About 2 MB in total.

use std::sync::OnceLock;

use crate::hyph::Hyphenator;

/// One face of an embeddable font family.
#[derive(Debug, Clone)]
pub struct FontFace {
    pub file_name: &'static str,
    pub bold: bool,
    pub italic: bool,
    pub data: &'static [u8],
}

#[derive(Debug, Clone)]
pub struct FontFamily {
    pub name: &'static str,
    pub faces: Vec<FontFace>,
}

pub(crate) static PT_SERIF_REGULAR: &[u8] = include_bytes!("../assets/fonts/PTF55F.ttf");
pub(crate) static PT_SERIF_ITALIC: &[u8] = include_bytes!("../assets/fonts/PTF56F.ttf");
pub(crate) static PT_SERIF_BOLD: &[u8] = include_bytes!("../assets/fonts/PTF75F.ttf");
pub(crate) static PT_SERIF_BOLD_ITALIC: &[u8] = include_bytes!("../assets/fonts/PTF76F.ttf");
pub(crate) static SANGHA: &[u8] = include_bytes!("../assets/fonts/sangha.ttf");
pub(crate) static MAIN_CSS: &str = include_str!("../assets/css/main.css");

static HYPH_SRC: &[(&str, &str)] = &[
    ("ru", include_str!("../assets/hyphenations/ru.txt")),
    ("uk", include_str!("../assets/hyphenations/uk.txt")),
    ("en", include_str!("../assets/hyphenations/en.txt")),
    ("de", include_str!("../assets/hyphenations/de.txt")),
];

/// Resources used by the converter. Cheap to share (`&Assets` / `Arc<Assets>`); hyphenation
/// dictionaries are parsed lazily on first use and cached.
pub struct Assets {
    fonts: Vec<FontFamily>,
    hyph: [OnceLock<Hyphenator>; 4],
    font_zip: OnceLock<Vec<u8>>,
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}

impl Assets {
    pub fn new() -> Assets {
        let fonts = vec![FontFamily {
            name: "PT Serif",
            faces: vec![
                FontFace {
                    file_name: "PTSerif-Regular.ttf",
                    bold: false,
                    italic: false,
                    data: PT_SERIF_REGULAR,
                },
                FontFace {
                    file_name: "PTSerif-Italic.ttf",
                    bold: false,
                    italic: true,
                    data: PT_SERIF_ITALIC,
                },
                FontFace {
                    file_name: "PTSerif-Bold.ttf",
                    bold: true,
                    italic: false,
                    data: PT_SERIF_BOLD,
                },
                FontFace {
                    file_name: "PTSerif-BoldItalic.ttf",
                    bold: true,
                    italic: true,
                    data: PT_SERIF_BOLD_ITALIC,
                },
            ],
        }];
        Assets {
            fonts,
            hyph: Default::default(),
            font_zip: OnceLock::new(),
        }
    }

    /// A process-wide shared instance.
    pub fn shared() -> &'static Assets {
        static A: OnceLock<Assets> = OnceLock::new();
        A.get_or_init(Assets::new)
    }

    /// Font family names for `GET /fonts` (`ConvertOptions::font_family` values).
    pub fn font_names(&self) -> Vec<String> {
        self.fonts.iter().map(|f| f.name.to_string()).collect()
    }

    pub fn font(&self, name: &str) -> Option<&FontFamily> {
        let n = name.trim();
        self.fonts.iter().find(|f| f.name.eq_ignore_ascii_case(n))
    }

    /// Base stylesheet (adapted from the Qt `xsl/css/style.css`).
    pub fn default_css(&self) -> &'static str {
        MAIN_CSS
    }

    /// Decorative drop-caps font of the Qt app (Sangha, has Latin and Cyrillic).
    pub fn dropcaps_font(&self) -> &'static [u8] {
        SANGHA
    }

    /// Languages with a hyphenation dictionary.
    pub fn hyphenation_languages(&self) -> Vec<&'static str> {
        HYPH_SRC.iter().map(|(l, _)| *l).collect()
    }

    /// Hyphenator for a language code (`ru`, `ru-RU`, `en_US`…); `None` if unsupported.
    pub fn hyphenator(&self, lang: &str) -> Option<&Hyphenator> {
        let l = lang.trim().to_ascii_lowercase();
        let base = l.split(['-', '_']).next().unwrap_or("");
        let i = HYPH_SRC.iter().position(|(code, _)| *code == base)?;
        Some(self.hyph[i].get_or_init(|| Hyphenator::from_patterns(HYPH_SRC[i].1)))
    }

    /// All embeddable fonts, deflated once into an in-memory zip (entries `OEBPS/fonts/<file>`),
    /// so conversions copy compressed data instead of re-compressing ~1.5 MB each time.
    pub(crate) fn font_archive(&self) -> &[u8] {
        self.font_zip.get_or_init(|| {
            use std::io::Write;
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let mut all: Vec<(&str, &[u8])> = self
                .fonts
                .iter()
                .flat_map(|f| f.faces.iter().map(|x| (x.file_name, x.data)))
                .collect();
            all.push(("Sangha.ttf", SANGHA));
            for (name, data) in all {
                if zw
                    .start_file(format!("OEBPS/fonts/{name}"), crate::epub::deflated())
                    .is_ok()
                {
                    let _ = zw.write_all(data);
                }
            }
            zw.finish().map(|c| c.into_inner()).unwrap_or_default()
        })
    }

    /// Italic face for series lines on generated covers.
    pub(crate) fn cover_italic(&self) -> &'static [u8] {
        PT_SERIF_ITALIC
    }

    pub(crate) fn cover_font(&self, bold: bool) -> &'static [u8] {
        if bold {
            PT_SERIF_BOLD
        } else {
            PT_SERIF_REGULAR
        }
    }
}
