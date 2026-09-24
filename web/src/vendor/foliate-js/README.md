# foliate-js (vendored)

Source: https://github.com/johnfactotum/foliate-js (MIT, see `LICENSE`).

Not published to npm under that name, so the minimal set of ES modules needed
to render a paginated EPUB is vendored here as plain files (no build step,
loaded by Vite as-is): `view.js`, `epub.js`, `epubcfi.js`, `overlayer.js`,
`text-walker.js`, `progress.js`, `paginator.js`, plus the bundled
`vendor/zip.js` (a build of `@zip.js/zip.js`, BSD-3-Clause, license in
`vendor/zip.js.LICENSE`) that `epub.js` uses to read the EPUB's zip archive.

Formats other than EPUB (`comic-book.js`, `fb2.js`, `mobi.js`, `pdf.js`,
fixed-layout, in-book search, TTS, …) were intentionally left out — freeLib
only needs a paginated EPUB reader in the browser. `view.js` is lightly
patched (each change is commented in place) so its `makeBook()`, `open()`,
`search()` and `initTTS()` no longer reference those unvendored modules —
Vite's dev transform doesn't reliably skip resolving a dynamic `import()`
just because it's marked `/* @vite-ignore */`, so the dead branches were
removed instead of ignored.

To refresh: re-copy the same files from the upstream repo at a pinned commit,
re-apply the same trims to `view.js` (diff against upstream's `open()`,
`makeBook()`, `search()`, `initTTS()`), and re-check its dynamic `import()`
list only points at files still vendored here.
