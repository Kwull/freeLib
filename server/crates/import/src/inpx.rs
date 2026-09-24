//! INPX reader: the zip container (`structure.info`, `version.info`, `collection.info`,
//! `*.inp` parts) and the `\x04`-separated record format.
//!
//! Behaviour follows the Qt importer (`freeLib/src/importthread.cpp`) with a few
//! deliberate differences, see `docs/web/ARCHITECTURE.md` ("INPX parsing").

use std::fs::File;
use std::io::Read;
use std::path::Path;

use zip::ZipArchive;

use crate::ImportError;

/// A known INPX field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Author,
    Genre,
    Title,
    Series,
    Serno,
    File,
    Size,
    LibId,
    Del,
    Ext,
    Date,
    Lang,
    Stars,
    Keywords,
    Folder,
    /// Anything else (`INSNO`, `URI`, `TAG`, …): ignored.
    Other,
}

impl Field {
    fn from_name(s: &str) -> Field {
        match s.trim().to_ascii_uppercase().as_str() {
            "AUTHOR" => Field::Author,
            "GENRE" => Field::Genre,
            "TITLE" => Field::Title,
            "SERIES" => Field::Series,
            "SERNO" => Field::Serno,
            "FILE" => Field::File,
            "SIZE" => Field::Size,
            "LIBID" => Field::LibId,
            "DEL" => Field::Del,
            "EXT" => Field::Ext,
            "DATE" => Field::Date,
            "LANG" => Field::Lang,
            "STARS" | "LIBRATE" => Field::Stars,
            "KEYWORDS" => Field::Keywords,
            "FOLDER" => Field::Folder,
            _ => Field::Other,
        }
    }
}

/// Field positions of a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Structure {
    idx: [Option<usize>; 15],
}

/// Field order used when the INPX has no `structure.info`.
pub const DEFAULT_STRUCTURE: &str = "AUTHOR;GENRE;TITLE;SERIES;SERNO;FILE;SIZE;LIBID;DEL;EXT;DATE;LANG;STARS;KEYWORDS;";

impl Default for Structure {
    fn default() -> Self {
        Structure::parse(DEFAULT_STRUCTURE)
    }
}

impl Structure {
    /// Parse `structure.info` (`AUTHOR;GENRE;…;` on the first non-empty line).
    /// Returns the default structure when the text contains no known field.
    pub fn parse(text: &str) -> Structure {
        let line = text.trim_start_matches('\u{feff}').lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        let mut idx = [None; 15];
        for (i, name) in line.split(';').enumerate() {
            let f = Field::from_name(name);
            if f != Field::Other && idx[f as usize].is_none() {
                idx[f as usize] = Some(i);
            }
        }
        if idx.iter().all(Option::is_none) && line != DEFAULT_STRUCTURE {
            return Structure::default();
        }
        Structure { idx }
    }

    /// Position of `f` in a record, if present.
    pub fn pos(&self, f: Field) -> Option<usize> {
        self.idx.get(f as usize).copied().flatten()
    }

    pub fn has(&self, f: Field) -> bool {
        self.pos(f).is_some()
    }
}

/// One author as written in INPX (`Last,First,Middle`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RawAuthor {
    pub last: String,
    pub first: String,
    pub middle: String,
}

/// Canonical author used for "unknown author" variants and books without authors.
pub const UNKNOWN_AUTHOR: &str = "Автор неизвестен";

impl RawAuthor {
    pub fn unknown() -> RawAuthor {
        RawAuthor { last: UNKNOWN_AUTHOR.into(), first: String::new(), middle: String::new() }
    }

    /// Display name `Last First Middle`, single-spaced.
    pub fn display(&self) -> String {
        freelib_catalog::normalize::collapse_ws(&format!("{} {} {}", self.last, self.first, self.middle))
    }
}

/// One parsed INPX record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawBook {
    /// Never empty: unknown / missing authors become [`RawAuthor::unknown`].
    pub authors: Vec<RawAuthor>,
    /// Canonical genre codes (lower-case, `_`), deduplicated, in INPX order.
    pub genres: Vec<String>,
    pub title: String,
    pub series: String,
    pub serno: Option<i64>,
    pub file: String,
    pub size: i64,
    pub lib_id: Option<i64>,
    pub deleted: bool,
    pub ext: String,
    /// `YYYY-MM-DD` or `""`.
    pub date: String,
    pub lang: String,
    pub stars: i64,
    pub keywords: String,
    /// Zip archive relative to the library folder, `""` for plain files.
    pub archive: String,
    /// Raw FOLDER value (`/` separators), `""` when absent.
    pub folder: String,
    /// Local header offset etc., filled when archives are indexed during import.
    pub loc: Option<crate::zipdir::ZipEntryLoc>,
}

impl RawBook {
    /// Stable key: `lib:<LIBID>` or `file:<archive or folder>/<file>.<ext>`.
    pub fn book_key(&self) -> String {
        match self.lib_id {
            Some(id) => format!("lib:{id}"),
            None => self.file_key(),
        }
    }

    /// The `file:` form of the key (also used when two records share a LIBID).
    pub fn file_key(&self) -> String {
        let base = if self.archive.is_empty() { self.folder.as_str() } else { self.archive.as_str() };
        file_key(base, &self.file, &self.ext)
    }

    /// Entry name inside the archive: `<file>.<ext>`.
    pub fn entry_name(&self) -> String {
        if self.ext.is_empty() { self.file.clone() } else { format!("{}.{}", self.file, self.ext) }
    }
}

/// `file:<base>/<file>.<ext>` (`file:<file>.<ext>` when `base` is empty).
pub fn file_key(base: &str, file: &str, ext: &str) -> String {
    let name = if ext.is_empty() { file.to_string() } else { format!("{file}.{ext}") };
    let base = base.trim_end_matches('/');
    if base.is_empty() { format!("file:{name}") } else { format!("file:{base}/{name}") }
}

/// Options that change which records / authors are kept.
#[derive(Debug, Clone, Copy, Default)]
pub struct ParseOptions {
    pub skip_deleted: bool,
    pub first_author_only: bool,
}

/// Archive for books of an `.inp` part: `fb2-000024-030559.inp` → `fb2-000024-030559.zip`.
pub fn archive_for_part(part_name: &str) -> String {
    let base = part_name.rsplit(['/', '\\']).next().unwrap_or(part_name);
    let stem = match base.rfind('.') {
        Some(i) => &base[..i],
        None => base,
    };
    format!("{stem}.zip")
}

/// Split the FOLDER field into `(archive, folder)`.
/// `x.zip` → archive `x.zip`; `x.inp` → archive `x.zip` (Qt compatibility);
/// anything else is a plain sub-folder (archive `""`).
fn resolve_folder(raw: &str, part_name: &str) -> (String, String) {
    let folder = raw.trim().replace('\\', "/");
    if folder.is_empty() {
        return (archive_for_part(part_name), String::new());
    }
    let lower = folder.to_ascii_lowercase();
    if lower.ends_with(".zip") {
        (folder.clone(), folder)
    } else if lower.ends_with(".inp") {
        (format!("{}.zip", &folder[..folder.len() - 4]), folder)
    } else {
        (String::new(), folder)
    }
}

fn is_unknown_author(s: &str) -> bool {
    let l = s.to_lowercase();
    let l = l.trim_matches(|c: char| c == ',' || c.is_whitespace());
    (l.contains("автор") && (l.contains("неизвестен") || l.contains("неизвестный"))) || l == "неизвестно" || l == "unknown"
}

/// Parse `Last,First,Middle:Last2,First2,:` (empty entries skipped, duplicates removed).
pub fn parse_authors(s: &str) -> Vec<RawAuthor> {
    let mut out: Vec<RawAuthor> = Vec::new();
    for part in s.split(':') {
        if part.trim().is_empty() || part.split(',').all(|p| p.trim().is_empty()) {
            continue;
        }
        let a = if is_unknown_author(part) {
            RawAuthor::unknown()
        } else {
            let mut it = part.split(',').map(freelib_catalog::normalize::collapse_ws);
            let last = it.next().unwrap_or_default();
            let first = it.next().unwrap_or_default();
            let middle = it.next().unwrap_or_default();
            // "First,,": only a first name in the last-name slot is fine; nothing to fix.
            RawAuthor { last, first, middle }
        };
        if !out.contains(&a) {
            out.push(a);
        }
    }
    if out.is_empty() {
        out.push(RawAuthor::unknown());
    }
    // An unknown author next to real ones carries no information.
    if out.len() > 1 {
        out.retain(|a| a.last != UNKNOWN_AUTHOR || !a.first.is_empty());
    }
    out
}

/// Parse `code1:code2:` into canonical, deduplicated codes.
pub fn parse_genres(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for g in s.split(':') {
        let c = freelib_catalog::genres::canonical_code(g);
        if !c.is_empty() && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

fn parse_int(s: &str) -> Option<i64> {
    let t = s.trim();
    if let Ok(v) = t.parse::<i64>() {
        return Some(v);
    }
    // Leading digits of e.g. "12a" or "3.5".
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Parse one record line. Returns `None` for empty/garbage lines and for deleted records
/// when `skip_deleted` is set.
pub fn parse_line(line: &str, part_name: &str, st: &Structure, opts: ParseOptions) -> Option<RawBook> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() || !line.contains('\x04') {
        return None;
    }
    let fields: Vec<&str> = line.split('\x04').collect();
    let get = |f: Field| -> &str { st.pos(f).and_then(|i| fields.get(i).copied()).unwrap_or("") };

    let deleted = parse_int(get(Field::Del)).unwrap_or(0) > 0;
    if deleted && opts.skip_deleted {
        return None;
    }
    let file = get(Field::File).trim().to_string();
    if file.is_empty() {
        return None;
    }
    let mut authors = parse_authors(get(Field::Author));
    if opts.first_author_only {
        authors.truncate(1);
    }
    let (archive, folder) = if st.has(Field::Folder) {
        resolve_folder(get(Field::Folder), part_name)
    } else {
        (archive_for_part(part_name), String::new())
    };
    let lang: String = get(Field::Lang).trim().to_lowercase().chars().take(2).collect();
    Some(RawBook {
        authors,
        genres: parse_genres(get(Field::Genre)),
        title: freelib_catalog::normalize::collapse_ws(get(Field::Title)),
        series: freelib_catalog::normalize::collapse_ws(get(Field::Series)),
        serno: parse_int(get(Field::Serno)).filter(|&n| n > 0),
        file,
        size: parse_int(get(Field::Size)).unwrap_or(0).max(0),
        lib_id: parse_int(get(Field::LibId)).filter(|&n| n > 0),
        deleted,
        ext: get(Field::Ext).trim().trim_start_matches('.').to_lowercase(),
        date: freelib_catalog::util::parse_date(get(Field::Date)),
        lang,
        stars: parse_int(get(Field::Stars)).unwrap_or(0).clamp(0, 5),
        keywords: freelib_catalog::normalize::collapse_ws(get(Field::Keywords)),
        archive,
        folder,
        loc: None,
    })
}

/// Parse a whole `.inp` part (UTF-8; invalid bytes are replaced).
pub fn parse_inp(data: &[u8], part_name: &str, st: &Structure, opts: ParseOptions) -> Vec<RawBook> {
    let text = String::from_utf8_lossy(data);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    text.split('\n').filter_map(|l| parse_line(l, part_name, st, opts)).collect()
}

/// Metadata of an INPX file.
#[derive(Debug, Clone, Default)]
pub struct InpxInfo {
    /// `version.info` content (trimmed), e.g. `20240501`.
    pub version: Option<String>,
    /// First line of `collection.info`.
    pub collection_name: Option<String>,
    /// Full `collection.info`.
    pub collection_info: Option<String>,
    pub structure: Structure,
    /// `.inp` entry names in archive order.
    pub parts: Vec<String>,
}

fn read_entry_text(za: &mut ZipArchive<File>, name: &str) -> Result<String, ImportError> {
    let mut f = za.by_name(name)?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).trim_start_matches('\u{feff}').to_string())
}

/// Open an INPX and read its metadata files (names matched case-insensitively).
pub fn read_info(path: &Path) -> Result<InpxInfo, ImportError> {
    let mut za = ZipArchive::new(File::open(path)?)?;
    let names: Vec<String> = za.file_names().map(str::to_string).collect();
    let find = |n: &str| names.iter().find(|x| x.eq_ignore_ascii_case(n)).cloned();
    let mut info = InpxInfo::default();
    if let Some(n) = find("structure.info") {
        info.structure = Structure::parse(&read_entry_text(&mut za, &n)?);
    }
    if let Some(n) = find("version.info") {
        let v = read_entry_text(&mut za, &n)?.split_whitespace().collect::<Vec<_>>().join(" ");
        info.version = (!v.is_empty()).then_some(v);
    }
    if let Some(n) = find("collection.info") {
        let t = read_entry_text(&mut za, &n)?;
        info.collection_name = t.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string);
        info.collection_info = Some(t);
    }
    info.parts = names.into_iter().filter(|n| n.to_ascii_lowercase().ends_with(".inp")).collect();
    Ok(info)
}

/// Read one `.inp` part's bytes.
pub fn read_part(za: &mut ZipArchive<File>, name: &str) -> Result<Vec<u8>, ImportError> {
    let mut f = za.by_name(name)?;
    let mut buf = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(fields: &[&str]) -> String {
        fields.join("\x04") + "\x04"
    }

    #[test]
    fn default_structure() {
        let st = Structure::default();
        assert_eq!(st.pos(Field::Author), Some(0));
        assert_eq!(st.pos(Field::Keywords), Some(13));
        assert_eq!(st.pos(Field::Folder), None);
    }

    #[test]
    fn custom_structure() {
        let st = Structure::parse("\u{feff}AUTHOR;GENRE;TITLE;SERIES;SERNO;FILE;SIZE;LIBID;DEL;EXT;DATE;INSNO;FOLDER;LANG;LIBRATE;KEYWORDS;\r\n");
        assert_eq!(st.pos(Field::Folder), Some(12));
        assert_eq!(st.pos(Field::Lang), Some(13));
        assert_eq!(st.pos(Field::Stars), Some(14));
        let st = Structure::parse("title;file;ext");
        assert_eq!(st.pos(Field::Title), Some(0));
        assert_eq!(st.pos(Field::Author), None);
        assert_eq!(Structure::parse(""), Structure::default());
    }

    #[test]
    fn full_record() {
        let st = Structure::default();
        let line = rec(&[
            "Стругацкий,Аркадий,Натанович:Стругацкий,Борис,Натанович:",
            "sf_social:sf:",
            " Пикник  на обочине ",
            "Миры Стругацких",
            "3",
            "12345",
            "456789",
            "12345",
            "0",
            "fb2",
            "2009-01-02",
            "RU",
            "4",
            "сталкер, зона",
        ]);
        let b = parse_line(&line, "fb2-000001-020000.inp", &st, ParseOptions::default()).unwrap();
        assert_eq!(b.authors.len(), 2);
        assert_eq!(b.authors[0].display(), "Стругацкий Аркадий Натанович");
        assert_eq!(b.genres, vec!["sf_social", "sf"]);
        assert_eq!(b.title, "Пикник на обочине");
        assert_eq!(b.series, "Миры Стругацких");
        assert_eq!(b.serno, Some(3));
        assert_eq!(b.size, 456789);
        assert_eq!(b.lib_id, Some(12345));
        assert!(!b.deleted);
        assert_eq!(b.ext, "fb2");
        assert_eq!(b.date, "2009-01-02");
        assert_eq!(b.lang, "ru");
        assert_eq!(b.stars, 4);
        assert_eq!(b.keywords, "сталкер, зона");
        assert_eq!(b.archive, "fb2-000001-020000.zip");
        assert_eq!(b.folder, "");
        assert_eq!(b.book_key(), "lib:12345");
        assert_eq!(b.file_key(), "file:fb2-000001-020000.zip/12345.fb2");
    }

    #[test]
    fn missing_and_odd_fields() {
        let st = Structure::default();
        // Only author, genre, title, series, serno, file: everything else missing.
        let line = "Иванов,,:\x04\x04Книга\x04\x04\x04777";
        let b = parse_line(line, "x.inp", &st, ParseOptions::default()).unwrap();
        assert_eq!(b.authors, vec![RawAuthor { last: "Иванов".into(), first: "".into(), middle: "".into() }]);
        assert!(b.genres.is_empty());
        assert_eq!(b.serno, None);
        assert_eq!(b.size, 0);
        assert_eq!(b.lib_id, None);
        assert_eq!(b.ext, "");
        assert_eq!(b.date, "");
        assert_eq!(b.book_key(), "file:x.zip/777");
        // CRLF, serno 0, bad numbers, bad date, stars out of range
        let line = rec(&["A,B,C:", "SF Heroic:", "T", "S", "0", "f", "abc", "0", "1", ".FB2", "2009-99-01", "en-US", "9", ""]) + "\r";
        let b = parse_line(&line, "x.inp", &st, ParseOptions::default()).unwrap();
        assert_eq!(b.genres, vec!["sf_heroic"]);
        assert_eq!(b.serno, None);
        assert_eq!(b.size, 0);
        assert_eq!(b.lib_id, None);
        assert!(b.deleted);
        assert_eq!(b.ext, "fb2");
        assert_eq!(b.date, "");
        assert_eq!(b.lang, "en");
        assert_eq!(b.stars, 5);
        // garbage lines
        assert!(parse_line("", "x.inp", &st, ParseOptions::default()).is_none());
        assert!(parse_line("no separators here", "x.inp", &st, ParseOptions::default()).is_none());
        // no FILE → skipped
        assert!(parse_line(&rec(&["A,,:", "sf:", "T"]), "x.inp", &st, ParseOptions::default()).is_none());
    }

    #[test]
    fn authors() {
        assert_eq!(parse_authors("").len(), 1);
        assert_eq!(parse_authors("")[0], RawAuthor::unknown());
        assert_eq!(parse_authors(",,:")[0], RawAuthor::unknown());
        assert_eq!(parse_authors("Автор Неизвестен,,:")[0], RawAuthor::unknown());
        assert_eq!(parse_authors("неизвестно")[0], RawAuthor::unknown());
        let a = parse_authors("Автор неизвестен:Иванов,Иван,Иванович:Иванов,Иван,Иванович:");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].display(), "Иванов Иван Иванович");
        let a = parse_authors("  Толстой , Лев ,Николаевич,extra:Dumas,Alexandre");
        assert_eq!(a[0], RawAuthor { last: "Толстой".into(), first: "Лев".into(), middle: "Николаевич".into() });
        assert_eq!(a[1].display(), "Dumas Alexandre");
    }

    #[test]
    fn options() {
        let st = Structure::default();
        let line = rec(&["A,,:B,,:", "sf:", "T", "", "", "1", "1", "1", "1", "fb2", "2020-01-01", "ru", "", ""]);
        assert!(parse_line(&line, "x.inp", &st, ParseOptions { skip_deleted: true, first_author_only: false }).is_none());
        let b = parse_line(&line, "x.inp", &st, ParseOptions { skip_deleted: false, first_author_only: true }).unwrap();
        assert_eq!(b.authors.len(), 1);
        assert_eq!(b.authors[0].last, "A");
    }

    #[test]
    fn folder_field() {
        let st = Structure::parse("AUTHOR;GENRE;TITLE;SERIES;SERNO;FILE;SIZE;LIBID;DEL;EXT;DATE;LANG;FOLDER;");
        let mk = |folder: &str| rec(&["A,,:", "sf:", "T", "", "", "10", "1", "", "0", "fb2", "2020-01-01", "ru", folder]);
        let b = parse_line(&mk("usr-1.zip"), "all.inp", &st, ParseOptions::default()).unwrap();
        assert_eq!((b.archive.as_str(), b.folder.as_str()), ("usr-1.zip", "usr-1.zip"));
        let b = parse_line(&mk("sub\\old-2.inp"), "all.inp", &st, ParseOptions::default()).unwrap();
        assert_eq!(b.archive, "sub/old-2.zip");
        let b = parse_line(&mk("books/plain"), "all.inp", &st, ParseOptions::default()).unwrap();
        assert_eq!((b.archive.as_str(), b.folder.as_str()), ("", "books/plain"));
        assert_eq!(b.book_key(), "file:books/plain/10.fb2");
        let b = parse_line(&mk(""), "all.inp", &st, ParseOptions::default()).unwrap();
        assert_eq!(b.archive, "all.zip");
    }

    #[test]
    fn archives() {
        assert_eq!(archive_for_part("fb2-000024-030559.inp"), "fb2-000024-030559.zip");
        assert_eq!(archive_for_part("dir/a.b.INP"), "a.b.zip");
        assert_eq!(archive_for_part("noext"), "noext.zip");
    }

    #[test]
    fn whole_part() {
        let st = Structure::default();
        let data = format!(
            "\u{feff}{}\n\n{}\r\n",
            rec(&["A,,:", "sf:", "One", "", "", "1", "1", "1", "0", "fb2", "2020-01-01", "ru", "", ""]),
            rec(&["B,,:", "sf:", "Two", "", "", "2", "1", "2", "0", "fb2", "2020-01-01", "ru", "", ""])
        );
        let v = parse_inp(data.as_bytes(), "p.inp", &st, ParseOptions::default());
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].title, "Two");
        // invalid UTF-8 does not abort the part
        let mut bytes = data.into_bytes();
        bytes.insert(10, 0xff);
        assert_eq!(parse_inp(&bytes, "p.inp", &st, ParseOptions::default()).len(), 2);
    }
}
