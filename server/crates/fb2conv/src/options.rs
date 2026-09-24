use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Hyphenate {
    /// No hyphenation (`hyphens: manual` in CSS).
    #[default]
    None,
    /// Soft hyphens inserted from the embedded dictionaries (ru, uk, en, de).
    Soft,
    /// Soft hyphens plus CSS `hyphens: auto`, so readers with their own dictionaries also
    /// hyphenate languages without an embedded dictionary.
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Footnotes {
    /// Notes file at the end with back links; `epub:type="noteref"` / `"footnote"`.
    #[default]
    End,
    /// Note text inserted into the paragraph, in brackets.
    Inline,
    /// `<aside epub:type="footnote">` in the notes file: pop-ups in Apple Books, Kobo, KOReader,
    /// Kindle (via Calibre/Send to Kindle).
    Popup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TocPlacement {
    Start,
    #[default]
    End,
    /// No visible table of contents page (the navigation document is still present).
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CreateCover {
    Never,
    #[default]
    Missing,
    Always,
}

/// Conversion options, JSON-compatible with `ConvertOptions` in `docs/web/API.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConvertOptions {
    pub hyphenate: Hyphenate,
    pub footnotes: Footnotes,
    pub drop_caps: bool,
    pub break_after_chapter: bool,
    pub toc_placement: TocPlacement,
    pub create_cover: CreateCover,
    /// Template drawn on the cover (`%s %n`, see [`crate::file_name`] placeholders).
    pub cover_label: Option<String>,
    pub join_series: bool,
    /// Used by callers for file names; the converter itself ignores it.
    pub transliterate: bool,
    pub annotation: bool,
    pub font_family: Option<String>,
    pub user_css: Option<String>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions {
            hyphenate: Hyphenate::None,
            footnotes: Footnotes::End,
            drop_caps: false,
            break_after_chapter: true,
            toc_placement: TocPlacement::End,
            create_cover: CreateCover::Missing,
            cover_label: None,
            join_series: false,
            transliterate: false,
            annotation: true,
            font_family: None,
            user_css: None,
        }
    }
}
