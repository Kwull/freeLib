# foliate-js (vendored)

Source: https://github.com/johnfactotum/foliate-js (MIT, see `LICENSE`).

Not published to npm under that name, so the minimal set of ES modules needed
to render a paginated EPUB is vendored here as plain files (no build step,
loaded by Vite as-is): `view.js`, `epub.js`, `epubcfi.js`, `overlayer.js`,
`text-walker.js`, `progress.js`, `paginator.js`, plus the bundled
`vendor/zip.js` (a build of `@zip.js/zip.js`, BSD-3-Clause, license in
`vendor/zip.js.LICENSE`) that `epub.js` uses to read the EPUB's zip archive.

Formats other than EPUB (`comic-book.js`, `fb2.js`, `mobi.js`, `pdf.js`, …)
were intentionally left out — freeLib only needs EPUB in the browser reader.

To refresh: re-copy the same files from the upstream repo at a pinned commit
and re-check `view.js`'s dynamic `import()` list hasn't grown new
dependencies for the EPUB path.
