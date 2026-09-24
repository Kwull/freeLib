# freeLib web

Svelte 5 + TypeScript + Vite single-page app for the freeLib web edition. Builds to
`web/dist`, which the Rust server (`server/`) embeds and serves.

## Running

```sh
pnpm install
pnpm dev        # http://localhost:5173, backed by the built-in mock API (web/mock/)
```

Against a real server instead of the mock:

```sh
VITE_API=http://localhost:8080 pnpm dev
```

With `VITE_API` set, Vite proxies `/api` and `/opds` to that server and the mock
middleware is not installed. Without it, `web/mock/server.ts` serves the full
`/api/v1` contract from `docs/web/API.md` using deterministic in-memory data
(50,000 authors, 4,000 series, ~3,000 books, jobs that progress over a few
seconds and stream through `/api/v1/events`), so the whole app — including the
50k-row authors list — can be exercised without the Rust server.

Other scripts:

```sh
pnpm build      # production build -> dist/
pnpm check      # svelte-check + tsc, no emit
pnpm test       # Playwright, against the mock (see tests/)
```

Playwright uses the Chromium already installed at `$PLAYWRIGHT_BROWSERS_PATH`;
don't run `playwright install`. `pnpm test` starts its own dev server on port
5183 (see `playwright.config.ts`).

## Structure

```
web/
  src/
    main.ts, App.svelte        entry point + route dispatch
    app.css                    design tokens (light/dark), global resets
    lib/
      api/                     typed client (client.ts), SSE (events.ts), types.ts
      i18n/                    en.json, ru.json, index.ts (t(), setLang)
      stores/                  *.svelte.ts — session, libraries, jobs, devices,
                                shelves, selection (per-library, in-memory),
                                theme, toast
      cache/nameCache.ts       IndexedDB cache for the authors/series lists,
                                keyed by catalogVersion
      router.svelte.ts         small history-based router (no SvelteKit)
      components/              Shell, NameBrowser, BooksPane, DetailsPane,
                                SendDialog, ShelfDialog, LibrariesPage's bits,
                                VirtualList (fixed-row-height virtualizer),
                                Icon (inline stroke-icon set), etc.
      routes/                  one component per SPA route
      utils/                   normalize() (mirrors the server's sort_key
                                normalization), formatting, file-name template
  mock/                        dev-only Vite middleware implementing docs/web/API.md
  tests/                       Playwright specs + tests/screenshots.spec.ts
  test-results/screenshots/    1440x900 and 390x844 screenshots for visual review
```

## Design

Colors, type (Literata + IBM Plex Sans, self-hosted via `@fontsource/*`), spacing
and icons follow `docs/web/prototype/*.dc.html`. Tokens live in `src/app.css` as
CSS variables on `:root`, redefined under `@media (prefers-color-scheme: dark)`
and `:root[data-theme="dark"]` for the manual toggle (Settings → General, or the
account menu).

Layout is responsive: below 900px width the shell switches to the phone pattern
from `Phone.dc.html` — a bottom tab bar, a stacked list → books → detail flow
(`/l/:lib/book/:id` is a dedicated route on phone), and a simplified one-line
book list instead of the desktop table (whose fixed columns don't fit a phone
screen).

## Performance

- The authors/series compact list is fetched once per `catalogVersion` (`?v=`)
  and cached in IndexedDB (`src/lib/cache/nameCache.ts`).
- Filtering the list as you type normalizes all rows once (on load, via an
  `$effect`, not per keystroke) and then does a single `includes()` pass; this
  keeps a 50,000-row filter around 5–15ms in practice (measured and logged to
  the console in dev when a filter pass exceeds 30ms). This was fast enough on
  the main thread that a Web Worker wasn't needed, per the budget in
  `docs/web/ARCHITECTURE.md`.
- Long lists (authors/series, book tables, cover grids) use a small
  fixed-row-height virtualizer (`VirtualList.svelte`) rather than a third-party
  dependency, since `@tanstack/svelte-virtual`'s Svelte 5 support was not
  something I wanted to gamble the perf budget on within this pass.
- Initial JS is ~66 KB gzipped; Settings and the reader are lazy-loaded
  (dynamic `import()` from `App.svelte`).
- The in-browser reader (`ReaderPage.svelte`) renders a real paginated EPUB
  via `foliate-js`, vendored (MIT) under `src/vendor/foliate-js` — see that
  folder's README for what was trimmed and why. Only the reader's own chunk
  (`view`/`epub`/`paginator`/`zip`, ~39 KB gzipped total) is fetched when a
  book is actually opened; it never touches the main bundle. The mock serves
  a real two-chapter Russian EPUB fixture for `/file?format=epub` (built by
  `web/mock/fixtures/build-epub.mjs`) so this is testable end to end.

## Known gaps / deviations

See the handback report for the full list. The short version: the vendored
reader only supports reflowable EPUB (no fixed-layout/CBZ/FB2/MOBI/PDF,
in-book search or TTS — see `src/vendor/foliate-js/README.md`); the mock's
SMTP/OPDS/import behavior is simulated, not real; and a handful of
`docs/web/API.md` details were assumed where the doc doesn't spell them out
(also listed in the handback report) — the mock implements those assumptions
explicitly in `web/mock/server.ts` so the server team can compare.
