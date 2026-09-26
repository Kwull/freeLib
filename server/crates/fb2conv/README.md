# freelib-fb2conv

FB2 metadata reading and FB2 → EPUB 3 / Kobo KEPUB conversion for the freeLib web edition.
It ports the EPUB branch of the Qt converter (`freeLib/src/fb2mobi/fb2mobi.cpp`,
`hyphenations.cpp`). AZW3/MOBI/PDF are produced by Calibre from this crate's EPUB in the server.

## Public API

```rust
// metadata (previews, cache/info)
read_info(bytes: &[u8]) -> Result<BookInfo>            // FB2 or .fb2.zip, any encoding
read_info_epub(bytes: &[u8]) -> Result<BookInfo>       // EPUB 2/3 (OPF metadata + cover)
read_info_any(bytes: &[u8]) -> Result<BookInfo>        // sniffs EPUB vs FB2/zipped FB2
sanitize_html(html: &str) -> Option<String>            // same sanitiser for foreign HTML

// conversion
fb2_to_epub(bytes: &[u8], opts: &ConvertOptions, assets: &Assets) -> Result<Vec<u8>>
fb2_to_epub_with(bytes, opts, assets, meta: &BookMeta) -> Result<Vec<u8>>   // + catalog metadata
join_to_epub(books: &[&[u8]], opts, assets, title: Option<&str>) -> Result<Vec<u8>>  // joinSeries
join_to_epub_with(books, opts, assets, title, meta: &BookMeta) -> Result<Vec<u8>>

// metadata helpers (also used for Calibre arguments)
BookMeta { book_key, lang, series, serno }   // the catalog's view; Default = FB2 only
book_uuid(book_key) -> "urn:uuid:…"          // stable EPUB identifier
normalize_language(tag) -> Option<String>    // BCP 47: "rus" → "ru", "ru_ru" → "ru-RU"
book_language(fb2_lang, fallback, sample)    // FB2 lang, else catalog lang, else a guess
clean_title(title, serno) / title_sort(title, lang)
to_kepub(epub: &[u8]) -> Result<Vec<u8>>               // save as *.kepub.epub
kepubify_xhtml(src: &str) -> Option<String>            // one document

// file names
file_name(template: &str, fields: &NameFields, transliterate: bool) -> String
expand_template(template: &str, fields: &NameFields) -> String   // cover labels, no FS sanitising
transliteration(s: &str) -> String

// resources
Assets::shared() -> &'static Assets    // or Assets::new(); cheap to share, lazily caches dictionaries
assets.font_names() -> Vec<String>     // GET /fonts → ["PT Serif"]
assets.hyphenator(lang) / hyphenation_languages() / default_css()
generate_cover(&Assets, author, title, bottom_line) -> Vec<u8>   // 1600×2560 JPEG (COVER_WIDTH/HEIGHT)
```

* `BookInfo` (serde, camelCase): `title, authors[{first,middle,last,nickname}], translators,
  series, serno, genres (FB2 codes / EPUB subjects), lang, annotation (HTML limited to
  <p>, <em>, <strong>, <br>), keywords, date, publisher, year, isbn, id` plus
  `cover: Option<CoverImage { data, mime }>`, which is not serialised. Cache the JSON and write
  the cover bytes separately.
* `ConvertOptions` has the JSON shape of `ConvertOptions` in `docs/web/API.md`. Missing fields
  take their defaults, so `{}` is valid. The converter ignores `transliterate` because it only
  applies to file names.
* `Error`: `Format` (not FB2 / broken container), `Zip`, `Io`.
* The `Doc`/`Builder` internals are private. Everything takes `&[u8]` and returns bytes, so
  callers decide about files, caching and threads. All functions are synchronous and CPU-bound,
  so the server should call them from `spawn_blocking` or a worker pool.

## Input handling

* **Encodings.** Checks for a BOM first (UTF-8, UTF-16LE/BE). Next it detects UTF-16 without a
  BOM, then reads `encoding="…"` from the XML declaration (any `encoding_rs` label:
  windows-1251, koi8-r, cp866, …). Input that claims UTF-8 but is invalid UTF-8 falls back to
  windows-1251, or to a lossy decode when only a few bytes are bad.
* **Zipped FB2.** A `PK\3\4` input is opened as a zip and its first `*.fb2` entry is used,
  with a 256 MB limit.
* **Broken markup.** A small tolerant DOM (`dom.rs`) sits on quick-xml:
  * HTML entities such as `&nbsp;` and `&laquo;` are resolved.
  * Unknown entities and stray `&` are kept as text.
  * Mismatched end tags close up to the matching open element; stray end tags are ignored.
  * Unclosed elements are closed at EOF, and unquoted attributes are accepted.
  * Namespace prefixes are dropped (`l:href`, `xlink:href` and `href` are all the same).
* **Fast `read_info`.** Only `<description>` is parsed into a tree. The cover `<binary>` is
  found by a text search after it, and its header is validated (dimensions) so broken covers
  come back as `None`. A 500 KB book takes about 1 ms.

## EPUB output

```
mimetype                                  (stored, first)
META-INF/container.xml
META-INF/com.apple.ibooks.display-options.xml   (only when fonts are embedded)
OEBPS/content.opf   EPUB 3.0 package: dc:identifier urn:uuid (from BookMeta.book_key, else the
                    FB2 document id or content hash; deterministic), cleaned dc:title with
                    title-type and file-as + calibre:title_sort, dcterms:modified, creators with
                    role/file-as, translators, description (plain text), subjects, publisher,
                    date (full date or year), ISBN, belongs-to-collection + collection-type +
                    group-position and calibre:series/series_index, <meta name="cover">,
                    cover-image property, legacy <guide> (docs/web/DEVICES.md)
OEBPS/toc.ncx       EPUB 2 NCX with nested navPoints (for older readers / Calibre / Kindle)
OEBPS/nav.xhtml     EPUB 3 nav (toc + hidden landmarks); also the visible TOC page
OEBPS/css/main.css  Qt style.css adapted for EPUB 3 + hyphenation/font rules + userCss
OEBPS/cover.xhtml   full-page SVG-wrapped cover (properties="svg")
OEBPS/annotation.xhtml, title.xhtml, part0001.xhtml …, notes.xhtml
OEBPS/img/…         images, OEBPS/fonts/… embedded fonts
```

* **Validation.** Every content document is XHTML5 with `xmlns`, `xmlns:epub`, `lang` and
  `xml:lang`. Ids are sanitised to XML names and made unique. Links whose target does not exist
  lose their `href`, because a dangling fragment is an epubcheck error.
* **epubcheck.** The tests run **epubcheck 5.2.1** on every generated EPUB and KEPUB when Java
  and the jar are present. The result is 0 errors and 0 warnings. The tests look for
  `EPUBCHECK_JAR` or `server/bench-data/epubcheck-*/epubcheck.jar`, and skip the check with a
  message when neither exists.
* **Structure.**
  * The main body's `<title>` becomes the title page (`<h1 class="titleblock h0">`) together with
    the body-level epigraphs and images. Without a body title, a title page is synthesised
    from the metadata.
  * Each top-level `<section>` starts a new file.
  * With `breakAfterChapter`, titled sections down to depth 3 also start new files.
  * Files are split before a section when they are over 160 KB, and between blocks when they
    are over 280 KB, so a single huge section still produces reasonably sized files.
  * Headings are real `h2…h6` elements with the Qt classes `titleblock hN`. Title lines are
    joined with `<br/>`.
  * The TOC nests sections by depth. The title page, the annotation page and the notes are
    top-level entries.
  * Additional unnamed `<body>`s are rendered in the main flow. Named bodies after the first are
    note bodies, as in Qt.
* **Elements.**

  | FB2 | XHTML |
  |---|---|
  | `epigraph` | `div.epigraph` |
  | `cite` | `blockquote.cite` |
  | `poem` / `stanza` / `v` | `div.poem` / `div.stanza` / `p.v`, with titles as `p.subtitle` |
  | `text-author` / `date` | `p.text-author` / `p.date` |
  | `subtitle` | `p.subtitle` |
  | `empty-line` | `p.empty-line` |
  | section `annotation` | `div.annotation` |
  | `table` | `table.table`; `colspan`/`rowspan` kept, `align`/`valign` become inline styles, because the attributes are obsolete in HTML5 |
  | `emphasis` / `strong` | `em` / `strong` |
  | `strikethrough` | `span.strike` |
  | `sub` / `sup` / `code` | `sub` / `sup` / `code` |
  | `style` | `span` (Qt dropped its text; we keep it) |

  Links are handled as follows:
  * Internal links are resolved to `file#id` after all files are written.
  * External `http(s)`/`mailto`/`ftp` links are kept.
  * Other links become spans.

  A leading dialogue dash gets a no-break space, which replaces Qt's `&#8197;`.
* **Images.** Images are decoded from `<binary>` with a lenient base64 decoder.
  * JPEG, PNG and GIF are kept after a header check.
  * WebP, BMP and anything else the `image` crate can decode are re-encoded: JPEG, or PNG when
    there is alpha. This keeps Kindle and older ADE readers happy.
  * Broken or missing images are dropped. An id on them is kept as an empty anchor.
* **Footnotes.**

  | Mode | Output |
  |---|---|
  | `end` | `<a class="anchor" epub:type="noteref">` links to `notes.xhtml`. Each note is a `<div class="note" epub:type="footnote">` whose number links back to its first reference |
  | `popup` | Same references, with notes as `<aside epub:type="footnote">`. Apple Books, Kobo, KOReader and Kindle show these as pop-ups (Apple Books hides asides in the flow) |
  | `inline` | Note text is inserted after the reference as `<span class="inlinenote">[…]</span>`; no notes file |

  Notes nested in untitled or id-less sections are found as in Qt, and unreferenced notes are
  still listed.
* **Covers.** `createCover` controls cover generation:
  * `missing`: the FB2 cover is used when it is valid; otherwise a typographic cover is drawn.
  * `always`: a cover is always drawn.
  * `never`: only the FB2 cover is used.

  The FB2 cover counts only when it decodes completely (a JPEG/PNG without its end marker is
  broken), is at least 100 px and not thinner than 1:4; covers much larger than 1600×2560 or
  heavier than 1.5 MB are scaled down to a JPEG. The drawn cover is 1600×2560 on a flat colour
  picked from the title, in PT Serif: the author at the top, the bold, line-balanced title in the
  middle (short words stay with the next one), the series in italics and "Книга N" (or the
  expanded `coverLabel`) at the bottom. Text shrinks until it fits. It is a 4:2:0 JPEG with
  optimised Huffman tables (`jpeg-encoder`), about 100–150 KB; the old leather texture
  (~330 KB at 780×1250) is gone. When `coverLabel` expands to a non-empty string, it is drawn
  top-right on a translucent box on existing covers, like Qt's "additional label".
* **Hyphenation.**
  * `soft` inserts U+00AD from the Qt Liang dictionaries (ru, uk, en, de), with a minimum of
    2 letters on each side. Headings, subtitles and code are not hyphenated.
  * `full` does the same and adds CSS `hyphens: auto`, so readers with their own dictionaries
    also cover other languages.
  * `none` sets `hyphens: manual`.
  * **Decision:** the Qt dictionaries (240 KB) are used instead of the `hyphenation` crate
    (several MB of embedded patterns) to keep the binary small and match the Qt app.
* **Drop caps.** The first letter of the first paragraph after a section title (main flow, not
  inside epigraphs or poems) is wrapped in `span.dropcaps`. The Qt default drop-caps font,
  Sangha, is embedded for it.
* **Fonts.** `fontFamily: "PT Serif"` embeds the four PT Serif faces as `font/ttf`, adds
  `@font-face` rules and `body { font-family }`, and adds the Apple display-options file.
  The fonts are deflated once per process into an in-memory archive and raw-copied into each
  EPUB.
* **`userCss`** is appended after the built-in stylesheet, so it overrides rules. Qt replaced
  the stylesheet instead.
* **`joinSeries`** (`join_to_epub`) produces:
  * a series title page, then each book's title page (with its annotation when `annotation`
    is set) and chapters one level deeper;
  * ids and images prefixed `b<N>_`, so identical ids in different books do not clash;
  * one notes file with a heading per book.

  The metadata title is the series name (or the `title` argument), and the authors are the
  union of all books' authors.

## KEPUB

`to_kepub` works like kepubify:
* Every sentence of body text is wrapped in `<span class="koboSpan" id="kobo.P.S">`, where P
  increments per block element, and images get their own span.
* The body content is wrapped in `<div id="book-columns"><div id="book-inner">`, and the
  kepubify style hack is added to `<head>`.
* `script`/`style`/`svg`/`math`/`pre` and the nav document are left alone. Other zip entries
  are raw-copied.
* It is idempotent: documents that already contain `koboSpan` are left unchanged.

## File names

`file_name("%a/%s/%n %b", &fields, translit)`, placeholders as in API.md:

| Placeholder | Value |
|---|---|
| `%a` | `Last F.` |
| `%fa` | full name |
| `%s` | series |
| `%n` | number, 2 digits (`%n3` = 3 digits, Qt syntax) |
| `%b` | title |
| `%l` | language |
| `%y` | year of `date` |

* `/` creates folders.
* **Missing values collapse with their separators.** Runs of ` - _ . , : ; # №` around a missing
  value shrink to one separator, and empty `()`/`[]`/`«»` groups disappear.
  `"%a - [%s #%n] %b"` without a series gives `"Стругацкий А. - Трудно быть богом"`.
* **Sanitising.**
  * `:` becomes `.`, `"` becomes `'` and `|` becomes `-`.
  * `?*<>\` and control characters are removed.
  * Leading dots, trailing dots on folder names (Windows) and spaces are trimmed.
  * Each part is limited to 200 bytes.
  * The result is never empty: it falls back to the title, then to `book`.
* **No extension.** The result has no extension; the caller appends one.
* **Transliteration** ports the Qt table: `ё→jo`, `ж→zh`, `х→h`, `ц→c`, `щ→sh`, `ы→i`,
  `ю→ju`, `я→ja`, and `ъ`/`ь` are dropped. It adds Ukrainian and Belarusian letters. Other
  non-ASCII characters are dropped, as in Qt.

## Qt options not carried over

| Qt option | Reason |
|---|---|
| Vignettes (image/text) | rarely used; they would add assets and pages |
| "Repair cover" (aspect fix with colour padding) | readers scale SVG-wrapped covers themselves |
| "Remove Personal tag", kindlegen, MOBI7, postprocessing tools | Calibre in the server does AZW3/MOBI |
| "Children" hyphenation (visible hyphens) | not in the web API |
| Footnotes "after paragraph" | not in the web API; `inline`/`popup` cover the use case |
| Split-file switch | files are always split, which is needed for reader performance; see `breakAfterChapter` |
| Multi-level TOC switch and max caption level | the TOC is always fully nested |
| Per-element fonts and sizes | the API has a single `fontFamily` (drop caps use Sangha) |
| Author string / book title templates for metadata (`%abbrs` etc.) | titles and authors come from the FB2 as is |

## Performance

These are release-build numbers on a 4-core container, averaged over 10 runs. They were
measured with `cargo run --release -p freelib-fb2conv --example fb2epub -- --synthetic 500 out.epub`,
which builds a 530 KB synthetic FB2 of random Russian words (about 40 chapters, notes and a
poem).

| Options | Time |
|---|---|
| no cover | 21 ms |
| defaults (generated 1600×2560 cover) | 95 ms (54 ms with the old 780×1250 cover) |
| + soft hyphenation | 80 ms |
| everything (hyphenation, popup notes, drop caps, PT Serif, forced cover with label) | 85 ms |
| a 2 MB FB2 with defaults | 110 ms |
| `read_info`, 530 KB | 1 ms |
| `to_kepub` of the 460 KB result | 22 ms |

The other rows with a cover were measured with the old 780×1250 cover; the new one adds about 40 ms.

Cover drawing costs about 70 ms at 1600×2560 (the result is cached per book and profile) and hyphenation about 30–40 ms per 500 KB. The synthetic book's EPUB is now 224 KB (was about 460 KB with the leather cover).

The example also converts real files:
`… --example fb2epub -- [--kepub] [--options '<json>'] in.fb2 out.epub`.

## Tests

`cargo test -p freelib-fb2conv`. The fixtures are in `tests/fixtures`:
* `full.fb2` covers all elements, notes, images (JPEG cover, PNG, GIF, a BMP that gets
  converted, a broken PNG and a missing image) and dangling links.
* `cp1251.fb2` is windows-1251 with a broken cover.
* `koi8.fb2.zip` is zipped KOI8-R, `utf16.fb2` is UTF-16 with a BOM, and `broken.fb2` has
  broken markup.
* `series1/2.fb2` are used for joining.

The tests check the structure of the generated XHTML, OPF, NCX and nav, check that every XML
file is well-formed and has unique ids, and run epubcheck when it is available.
