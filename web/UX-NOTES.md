# UX notes (usability review, 2026-09)

Reviewed on a 100k-book synthetic Flibusta-like library (`gen-inpx --books 100000`: Zipf-like
authors, "Азимов Айзек" with ~900 books / 100 series / 400+ anthologies, Latin names with
diacritics, long titles) against the real server, 1440×900 and 390×844, light and dark, RU UI.

## Findings → changes

**Authors / series browsing**
- Co-author line listed every name sharing an anthology (thousands, no spaces) and pushed the
  books below the fold → header is crumb + name + one counts line + one co-author line: top 3
  (real co-authors first, then ≥ 2 shared books) and "and N more", a searchable popover
  (`GET …/authors/:id/coauthors`). Counts come from the new `…/summary` endpoint.
- Letter strip mixed # A–Z, accented Latin and А–Я → sort keys fold Latin diacritics
  (server + SPA `normalize`, shared vectors); one alphabet at a time with an А–Я / A–Z / #
  switch (main script of the library first; follows scrolling); empty letters dimmed/disabled.
- Digit/symbol names came first → `#` group at the end; the list opens at the main script
  (А for a Russian library) or scrolls to the selected name. Authors/series with no live books
  are not listed (were shown with 0). Breadcrumb uses the index letter (`#` for "50languages").
- Genres: 322 genres in one long list → groups fold (current one open), filter box, empty
  genres hidden by default.

**Book table**
- Title column was ~70px at 1440 (empty details pane + fixed columns) → the details pane is hidden
  until a book is picked (outside author pages), rating column is optional (off by default, and
  shows stars only when rated), panes and columns are resizable.
- ~1000 books / 100 series were unwieldy → "Find in these books", sort (series / title / newest),
  groups start collapsed above 8 series with expand/collapse all and per-group counts and a link
  to the series, "Hide anthologies (N)" (books with ≥ 4 authors), anthology/deleted tags in rows,
  `N of M` when filtered; one group (or no series) → no group header.
- Filter menu (language, format, show deleted) replaces the ambiguous "Hide deleted" button;
  empty results offer "Reset filters"; load errors offer "Retry"; skeleton while loading.
- Keyboard: ↑/↓ move the current book, Space ticks it, ← folds its group. Cover grid pages in
  more books (genre/new arrivals) instead of stopping at the first page.
- Visible columns, sort, view, anthology toggle persist per user.

**Resizing** — nav, list, details panes and table columns: drag handles (hover line,
`col-resize`), focusable `role=separator` with arrows (Shift = ×4), Home/End, Enter/double-click
= default; min/max per pane/column; panes shrink proportionally with the window (flex-basis),
off below 900px. Header and rows share one grid template; the title column takes the rest (a
handle right of the title resizes the column to its right). Widths live in `/me/prefs`
(debounced) with localStorage for the first paint. The details pane collapses to a rail.

**Details pane** — author summary when nothing is selected (counts, years, languages, top
genres, co-authors, series with counts → links). Long titles get a smaller size and span the
pane; anthologies list 3 authors + "and N more"; the send button gets its own row when narrow.

**Search** — typeahead has ↑/↓/Enter, shows authors of books, ignores late answers; a book opens
in its author's list (`/l/:lib/authors/:id?book=:id`, group expanded and scrolled to). Results
page: editions of the same work (title + authors) collapse into one row with "+N editions",
book details open beside the results, facets fold behind "Filters" on the phone, top 8 genres
+ "show all", loading/empty states with hints, localized dates.

**Sending** — "Send to <last used device>"; the dialog preselects it, focuses the send button
(Enter sends), remembers the e-mail address typed per device, blocks sending without one with a
hint, has a sticky footer (was cut off on the phone). Finished jobs from this tab are announced
by a toast, downloads start by themselves, failures show the error.

**Other** — New arrivals: week / month / 3 months / year presets + date, book count.
Libraries: locale number format, one "Re-import" (the two modes ran the same import),
"Open", error state, error toasts. Settings: device list shows the address (or that it is
missing), localized device editor, "Default library" select actually saves, delete confirmations,
phone layout. Shelves: explanation when empty, delete confirmation, "My shelves" label hidden
when there are none. Phone: top bar no longer pushes Activity/account off screen, toasts above
the tab bar, phone back arrow. Dark theme: toasts and the selection bar were white-on-white,
ticked rows were light in system-dark mode → inverse / row-checked tokens. Login: autocomplete
and autofocus.

## Not changed (noted)
- `newSinceLastVisit` has no matching date filter in the API, so New arrivals uses presets.
- The reader (foliate-js) logs a benign "ResizeObserver loop" in dev.
