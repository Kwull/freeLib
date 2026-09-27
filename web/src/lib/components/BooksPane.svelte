<script lang="ts">
  import { tick, untrack } from 'svelte';
  import type { Book, Coauthor, Edition } from '../api/types';
  import FollowButton from './FollowButton.svelte';
  import { api, errorText } from '../api/client';
  import Icon from './Icon.svelte';
  import CoverThumb from './CoverThumb.svelte';
  import Rating from './Rating.svelte';
  import SelectionBar from './SelectionBar.svelte';
  import VirtualList from './VirtualList.svelte';
  import Splitter from './Splitter.svelte';
  import CoauthorsPopover from './CoauthorsPopover.svelte';
  import ExtRating from './ExtRating.svelte';
  import KidsBadge from './KidsBadge.svelte';
  import RatingFilters from './RatingFilters.svelte';
  import {
    emptyRatingFilters, ratingFilterCount, matchesRatingFilters, sortByRating, ratingParams, formatAvg,
    type RatingFilters as RatingFiltersT, type RatingSortKey,
  } from '../utils/ratings';
  import { formatSize, formatDate } from '../utils/format';
  import { normalize } from '../utils/normalize';
  import { dismissable } from '../utils/dismiss';
  import { popover } from '../utils/popover';
  import { t, tn, i18nState } from '../i18n';
  import { getPref, setPref } from '../stores/prefs.svelte';
  import { myRatings } from '../stores/myRatings.svelte';
  import { COL_LIMITS, colWidth, setColWidth, setDetailsCollapsed, type ColKey } from '../stores/layout.svelte';
  import {
    isSelected, selectedCount, toggle, toggleMany, clear as clearSelection, selectedIds,
  } from '../stores/selection.svelte';

  type Scope =
    | { kind: 'author'; id: number; groupable: true }
    | { kind: 'series'; id: number; groupable: false }
    | { kind: 'genre'; id: number; groupable: false }
    | { kind: 'shelf'; id: number; groupable: false }
    | { kind: 'since'; date: string; groupable: false };

  type Header = {
    crumb: string;
    name: string;
    /** works (list rows with editions grouped) */
    booksCount: number;
    /** live files, shown when they differ from the works */
    filesCount?: number;
    seriesCount?: number;
    anthologies?: number;
    /** top co-authors of an author (see AuthorSummary) */
    coauthors?: Coauthor[];
    coauthorCount?: number;
    /** a Follow button for this author / series */
    follow?: { kind: 'author' | 'series'; id: number };
  };

  let {
    lib, scope, selectedBookId, onPick, onOpenSend, onOpenShelf, header, onBack, onCounts, onSendSeries,
  }: {
    lib: number;
    scope: Scope;
    selectedBookId: number | null;
    onPick: (id: number) => void;
    onOpenSend: (ids: number[]) => void;
    onOpenShelf: (ids: number[]) => void;
    header?: Header;
    onBack?: () => void;
    onCounts?: (counts: { books: number }) => void;
    /** "Send whole series" (series pages, and the selection bar for selected books in series) */
    onSendSeries?: (seriesIds: number[]) => void;
  } = $props();

  /** A book with this many authors is an anthology / collection (same as the server). */
  const ANTHOLOGY_MIN_AUTHORS = 4;
  /** More series groups than this start collapsed. */
  const COLLAPSE_ABOVE = 8;
  const ROW_H = 40;

  const OPT_COLUMNS = ['author', 'series', 'genre', 'language', 'format', 'size', 'added', 'rating', 'libRating', 'extRating'] as const;
  const RATING_COLS = ['rating', 'libRating', 'extRating'] as const;
  type OptCol = (typeof OPT_COLUMNS)[number];
  type SortKey = 'series' | 'number' | 'title' | 'date' | RatingSortKey;
  const isRatingSort = (k: SortKey): k is RatingSortKey => k === 'myRating' || k === 'libRating' || k === 'extRating';

  const scopeKey = $derived(scope.kind === 'since' ? `since:${scope.date}` : `${scope.kind}:${scope.id}`);
  const fullScope = $derived(scope.kind === 'author' || scope.kind === 'series');
  // Author/series scopes are loaded in full (a whole bibliography, at most a few thousand
  // books) and filtered/sorted here; genre/new-arrivals/shelf scopes can run into the tens of
  // thousands of rows, so those page from the server as the list scrolls.
  const paginated = $derived(!fullScope);

  let books = $state<Book[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let reloadTick = $state(0);
  let nextCursor = $state<string | null>(null);
  let fetchingMore = $state(false);
  let total = $state<number | null>(null);

  let genreNames = $state<Map<number, string>>(new Map());
  $effect(() => { const lang = i18nState.lang; api.genres(lib, lang).then((gs) => (genreNames = new Map(gs.map((g) => [g.id, g.name])))).catch(() => {}); });

  // ---- view options (persisted per user) -------------------------------------------
  const colsPrefKey = $derived(scope.kind === 'author' ? 'cols.author' : scope.kind === 'series' ? 'cols.series' : 'cols.other');
  // Entries are column keys, `!size` / `!added` for a default column the user hid, and
  // `+key` for a column the user asked for while it had no room (it then stays, and the table
  // scrolls sideways). Every other column gives way when the books pane is too narrow, so a
  // list saved by an older version (plain keys only) fits the pane too.
  const extraColumns = $derived(
    new Set<string>(getPref<string[]>(colsPrefKey, scope.kind === 'author' ? [] : ['author'])),
  );
  /** Size and Added are on by default, except when rating columns are on (the table then
   *  fits beside the details pane); an explicit choice in the Columns menu wins. */
  function shownIn(set: Set<string>, c: OptCol): boolean {
    if (c === 'size' || c === 'added') {
      if (set.has(c)) return true;
      if (set.has(`!${c}`)) return false;
      return !RATING_COLS.some((k) => set.has(k));
    }
    return set.has(c);
  }
  const colShown = (c: OptCol) => shownIn(extraColumns, c);
  const view = $derived(getPref<'table' | 'grid'>('booksView', 'table'));
  // author / series lists are sorted here; genre, new arrivals and shelves by the server
  const sortPrefKey = $derived(`sort.${scope.kind === 'author' ? 'author' : scope.kind === 'series' ? 'series' : 'paged'}`);
  const sort = $derived<SortKey>(
    scope.kind === 'author' ? getPref<SortKey>(sortPrefKey, 'series')
      : scope.kind === 'series' ? getPref<SortKey>(sortPrefKey, 'number')
        : getPref<SortKey>(sortPrefKey, 'date'),
  );
  const hideAnth = $derived(scope.kind === 'author' && getPref<boolean>('hideAnthologies', false));
  /** one row per work (editions of the same title by the same authors), the best copy shown */
  const groupEditions = $derived(getPref<boolean>('groupEditions', true));
  /** grouped rows whose other editions are shown, and the editions loaded for them */
  let openEditions = $state<Set<number>>(new Set());
  let editionsOf = $state<Map<number, Edition[]>>(new Map());
  function toggleEditions(id: number) {
    const s = new Set(openEditions);
    if (s.has(id)) { s.delete(id); openEditions = s; return; }
    s.add(id);
    openEditions = s;
    if (!editionsOf.has(id)) {
      api.editions(lib, id)
        .then((r) => (editionsOf = new Map(editionsOf).set(id, r.books)))
        .catch(() => { const o = new Set(openEditions); o.delete(id); openEditions = o; });
    }
  }

  let showDeleted = $state(false);
  let ratingFilters = $state<RatingFiltersT>(emptyRatingFilters());
  let langFilter = $state<string | null>(null);
  let extFilter = $state<string | null>(null);
  let text = $state('');
  let q = $state('');
  let qTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const v = text;
    clearTimeout(qTimer);
    qTimer = setTimeout(() => (q = v.trim()), paginated ? 300 : 120);
  });
  let columnMenuOpen = $state(false);
  let filterMenuOpen = $state(false);
  let filterBtn = $state<HTMLButtonElement | undefined>();
  let columnBtn = $state<HTMLButtonElement | undefined>();
  let coauthorsBtn = $state<HTMLButtonElement | undefined>();
  let coauthorsOpen = $state(false);
  // Folded series groups: a default for every group (also ones that appear later, as more
  // books load) and the groups the user flipped away from it.
  let foldByDefault = $state(false);
  let flipped = $state<Set<string>>(new Set());
  let collapseInitFor = '';
  let lastClickedIndex = -1;
  let listRef = $state<{ reveal: (i: number) => void; resetX: () => void; scrollLeft: () => number } | undefined>();
  let mobileListRef = $state<{ reveal: (i: number) => void } | undefined>();
  let tableScroll = $state<HTMLDivElement | undefined>();
  let mobileWrap = $state<HTMLDivElement | undefined>();
  let tableWrap = $state<HTMLDivElement | undefined>();
  /** the table's width, for dropping columns that do not fit */
  let tableWidth = $state(0);
  $effect(() => {
    const el = tableWrap;
    if (!el) return;
    let raf = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => (tableWidth = el.clientWidth));
    });
    tableWidth = el.clientWidth;
    ro.observe(el);
    return () => { cancelAnimationFrame(raf); ro.disconnect(); };
  });
  // horizontal scroll of a wide table: the header follows the rows
  let headWrap = $state<HTMLDivElement | undefined>();
  let scrolledX = $state(false);
  function onScrollX(left: number) {
    if (headWrap) headWrap.scrollLeft = left;
    scrolledX = left > 0;
  }

  // New scope: forget the per-scope filters.
  $effect(() => {
    scopeKey;
    untrack(() => {
      text = ''; q = ''; langFilter = null; extFilter = null; showDeleted = false;
      ratingFilters = emptyRatingFilters();
      coauthorsOpen = false; filterMenuOpen = false; columnMenuOpen = false;
      openEditions = new Set(); editionsOf = new Map();
    });
  });

  function queryParams(): Record<string, unknown> {
    const params: Record<string, unknown> = { deleted: showDeleted, group: groupEditions };
    if (scope.kind === 'author') params.author = scope.id;
    else if (scope.kind === 'series') params.series = scope.id;
    else if (scope.kind === 'genre') params.genre = scope.id;
    else if (scope.kind === 'shelf') params.shelf = scope.id;
    else params.since = scope.date;
    if (langFilter) params.lang = langFilter;
    if (extFilter) params.ext = extFilter;
    if (paginated && q) params.q = q;
    // big scopes are filtered and sorted by rating on the server
    if (paginated) Object.assign(params, ratingParams(ratingFilters, isRatingSort(sort) ? sort : null));
    return params;
  }

  $effect(() => {
    reloadTick;
    const params = queryParams();
    const isPaginated = paginated;
    const ctrl = new AbortController();
    loading = true;
    loadError = null;
    hay = new Map();
    books = [];
    nextCursor = null;
    total = null;
    (async () => {
      try {
        if (isPaginated) {
          const res = await api.books(lib, { ...params, limit: 100 } as any, { signal: ctrl.signal });
          books = res.books;
          nextCursor = res.nextCursor;
          total = res.total;
          return;
        }
        let cursor: string | undefined;
        const acc: Book[] = [];
        for (let page = 0; page < 25; page++) {
          const res = await api.books(lib, { ...params, cursor, limit: 2000 } as any, { signal: ctrl.signal });
          acc.push(...res.books);
          books = acc.slice();
          total = res.total;
          if (!res.nextCursor) break;
          cursor = res.nextCursor;
        }
      } catch (e) {
        if (!ctrl.signal.aborted) loadError = errorText(e);
      } finally {
        if (!ctrl.signal.aborted) loading = false;
      }
    })();
    return () => ctrl.abort();
  });

  // ratings changed elsewhere (details pane): update the loaded rows
  $effect(() => {
    const changed = myRatings.changed;
    untrack(() => {
      let any = false;
      const next = books.map((b) => {
        const r = changed[`${lib}:${b.id}`];
        if (r !== undefined && r !== b.rating) { any = true; return { ...b, rating: r }; }
        return b;
      });
      if (any) books = next;
    });
  });

  async function loadMore() {
    if (!paginated || !nextCursor || fetchingMore) return;
    fetchingMore = true;
    try {
      const res = await api.books(lib, { ...queryParams(), cursor: nextCursor, limit: 100 } as any);
      books = [...books, ...res.books];
      nextCursor = res.nextCursor;
    } catch { /* the next scroll retries */ } finally {
      fetchingMore = false;
    }
  }

  function onRowRangeChange(_start: number, end: number) {
    if (end >= flatRows.length - 20) loadMore();
  }

  // ---- client-side filter / sort for full scopes -----------------------------------
  let hay = new Map<number, string>();
  function haystack(b: Book): string {
    let h = hay.get(b.id);
    if (h === undefined) {
      h = normalize(`${b.title} ${b.series?.name ?? ''} ${b.authors.map((a) => a.name).join(' ')}`);
      hay.set(b.id, h);
    }
    return h;
  }
  const isAnthology = (b: Book) => b.authors.length >= ANTHOLOGY_MIN_AUTHORS;

  const textMatched = $derived.by(() => {
    if (paginated || !q) return books;
    const words = normalize(q).split(' ').filter(Boolean);
    if (!words.length) return books;
    return books.filter((b) => { const h = haystack(b); return words.every((w) => h.includes(w)); });
  });
  const anthCount = $derived(scope.kind === 'author' ? textMatched.filter(isAnthology).length : 0);
  const visibleBooks = $derived.by(() => {
    let list = hideAnth ? textMatched.filter((b) => !isAnthology(b)) : textMatched;
    if (paginated) return list;
    if (ratingFilterCount(ratingFilters)) list = list.filter((b) => matchesRatingFilters(b, ratingFilters));
    if (isRatingSort(sort)) return sortByRating(list, sort);
    if (sort === 'title') list = list.slice().sort((a, b) => cmpStr(normalize(a.title), normalize(b.title)) || a.id - b.id);
    else if (sort === 'date') list = list.slice().sort((a, b) => cmpStr(b.date, a.date) || cmpStr(a.title, b.title));
    return list;
  });
  function cmpStr(a: string, b: string) { return a < b ? -1 : a > b ? 1 : 0; }

  type Row = { book: Book; num: number | string };
  type Group = { key: string; name: string; seriesId: number | null; count: number; rows: Row[] };

  const grouped = $derived(scope.kind === 'author' && sort === 'series');
  const groups = $derived.by<Group[]>(() => {
    const numFor = (b: Book, i: number) => (fullScope ? (b.serno ?? '') : i + 1);
    if (!grouped) {
      return [{ key: '_all', name: '', seriesId: null, count: visibleBooks.length, rows: visibleBooks.map((b, i) => ({ book: b, num: numFor(b, i) })) }];
    }
    const bySeries = new Map<string, Book[]>();
    const order: string[] = [];
    for (const b of visibleBooks) {
      const key = b.series ? `s${b.series.id}` : '_none';
      if (!bySeries.has(key)) { bySeries.set(key, []); order.push(key); }
      bySeries.get(key)!.push(b);
    }
    // books outside any series always come last
    const none = order.indexOf('_none');
    if (none >= 0) order.push(...order.splice(none, 1));
    return order.map((key) => {
      const list = bySeries.get(key)!;
      const s = key === '_none' ? null : list[0].series!;
      return {
        key, name: s ? s.name : t('books.outsideSeries'), seriesId: s ? s.id : null, count: list.length,
        rows: list.map((b) => ({ book: b, num: b.serno ?? '' })),
      };
    });
  });
  const showGroupHeads = $derived(groups.length > 1);

  // Many series: start with all groups collapsed (a table of contents); a text filter opens them.
  $effect(() => {
    const n = groups.length;
    if (loading || !books.length) return;
    if (collapseInitFor === scopeKey) return;
    collapseInitFor = scopeKey;
    foldByDefault = n > COLLAPSE_ABOVE;
    flipped = new Set();
  });
  const collapsed = $derived(new Set(groups.filter((g) => foldByDefault !== flipped.has(g.key)).map((g) => g.key)));
  const effectiveCollapsed = $derived(q ? new Set<string>() : collapsed);
  /** "Collapse all" only when every group is open; otherwise "Expand all" */
  const allExpanded = $derived(groups.every((g) => !effectiveCollapsed.has(g.key)));

  function setCollapsed(key: string, on: boolean) {
    const s = new Set(flipped);
    if (on !== foldByDefault) s.add(key); else s.delete(key);
    flipped = s;
  }
  function toggleCollapse(key: string) {
    setCollapsed(key, !collapsed.has(key));
  }
  function setAllCollapsed(on: boolean) {
    foldByDefault = on;
    flipped = new Set();
  }

  // A book selected from outside (search, deep link): open its group and scroll to it.
  let revealedFor: number | null = null;
  $effect(() => {
    const id = selectedBookId;
    if (id === null || loading) return;
    untrack(() => {
      if (revealedFor === id) return;
      const g = groups.find((gr) => gr.rows.some((r) => r.book.id === id));
      if (!g) return;
      revealedFor = id;
      if (collapsed.has(g.key)) setCollapsed(g.key, false);
      tick().then(() => {
        const i = flatRows.findIndex((fr) => fr.kind === 'row' && fr.row.book.id === id);
        if (i >= 0) listRef?.reveal(Math.max(0, i));
      });
    });
  });

  type FlatRow = { kind: 'group'; group: Group } | { kind: 'row'; row: Row; groupKey: string }
    | { kind: 'edition'; ed: Edition; of: number; ofTitle: string };
  const flatRows = $derived.by<FlatRow[]>(() => {
    const out: FlatRow[] = [];
    for (const g of groups) {
      if (showGroupHeads) out.push({ kind: 'group', group: g });
      if (showGroupHeads && effectiveCollapsed.has(g.key)) continue;
      for (const r of g.rows) {
        out.push({ kind: 'row', row: r, groupKey: g.key });
        if (openEditions.has(r.book.id)) {
          for (const ed of editionsOf.get(r.book.id) ?? []) if (ed.id !== r.book.id) out.push({ kind: 'edition', ed, of: r.book.id, ofTitle: r.book.title });
        }
      }
    }
    return out;
  });
  function editionLine(e: Edition): string {
    return [e.ext.toUpperCase(), formatSize(e.size), formatDate(e.date, i18nState.lang), e.lang, e.libRating ? `★${e.libRating}` : '', e.note ?? '']
      .filter(Boolean).join(' · ');
  }
  /** An edition's own title is shown when it is not just the group title (another translation
   *  under another title, an omnibus); edition notes are ignored in the comparison. */
  function differentTitle(e: Edition, groupTitle: string): boolean {
    const bare = (t: string) => normalize(t.replace(/\s*[([][^)\]]*[)\]]\s*$/u, ''));
    return bare(e.title) !== bare(groupTitle);
  }

  $effect(() => {
    if (!onCounts) return;
    onCounts({ books: total ?? books.length });
  });

  const availableLangs = $derived.by(() => [...new Set(books.map((b) => b.lang).filter(Boolean))].sort());
  const availableExts = $derived.by(() => [...new Set(books.map((b) => b.ext).filter(Boolean))].sort());
  const activeFilterCount = $derived((langFilter ? 1 : 0) + (extFilter ? 1 : 0) + (showDeleted ? 1 : 0) + ratingFilterCount(ratingFilters));
  const anyFilter = $derived(activeFilterCount > 0 || !!q || hideAnth);

  function resetFilters() {
    text = ''; q = ''; langFilter = null; extFilter = null; showDeleted = false;
    ratingFilters = emptyRatingFilters();
    if (hideAnth) setPref('hideAnthologies', false);
  }

  const allIds = $derived(visibleBooks.map((b) => b.id));
  const allChecked = $derived(allIds.length > 0 && allIds.every((id) => isSelected(lib, id)));

  function toggleAll() {
    toggleMany(lib, allIds, !allChecked);
  }
  function toggleGroup(ids: number[]) {
    const on = !ids.every((id) => isSelected(lib, id));
    toggleMany(lib, ids, on);
  }
  function rowClick(e: MouseEvent, book: Book, flatIndex: number) {
    if (e.shiftKey && lastClickedIndex >= 0) {
      const [a, b] = [lastClickedIndex, flatIndex].sort((x, y) => x - y);
      const ids = flatRows.slice(a, b + 1).flatMap((fr) => (fr.kind === 'row' ? [fr.row.book.id] : []));
      toggleMany(lib, ids, true);
    } else {
      onPick(book.id);
    }
    lastClickedIndex = flatIndex;
    activeKey = keyOf(flatRows[flatIndex]);
  }

  // ---- keyboard: the ARIA grid pattern over the flat rows ---------------------------
  // ↑/↓ move the current row (group headers and edition rows included), Home/End/PgUp/PgDn
  // jump, Enter opens (the details pane, reopened if folded; the book page on phones), Space
  // ticks the checkbox, ←/→ fold/unfold a series group or a work's editions. The current row
  // is kept by identity (not index), so folding a group above it does not move it.
  let activeKey = $state<string | null>(null);
  const keyOf = (fr: FlatRow | undefined): string | null =>
    !fr ? null : fr.kind === 'group' ? `g:${fr.group.key}` : fr.kind === 'row' ? `r:${fr.row.book.id}` : `e:${fr.of}:${fr.ed.id}`;
  const activeIndex = $derived.by(() => {
    if (activeKey !== null) {
      const i = flatRows.findIndex((fr) => keyOf(fr) === activeKey);
      if (i >= 0) return i;
    }
    return selectedBookId === null ? -1
      : flatRows.findIndex((fr) => (fr.kind === 'row' && fr.row.book.id === selectedBookId) || (fr.kind === 'edition' && fr.ed.id === selectedBookId));
  });
  // a book picked elsewhere (search, deep link, the details pane) becomes the current row
  $effect(() => {
    const id = selectedBookId;
    untrack(() => {
      const cur = flatRows[activeIndex];
      const curId = cur?.kind === 'row' ? cur.row.book.id : cur?.kind === 'edition' ? cur.ed.id : null;
      if (id !== null && curId !== id) activeKey = null;
    });
  });
  const rowDomId = (i: number) => `books-${lib}-row-${i}`;

  function moveTo(i: number) {
    const rows = flatRows;
    if (!rows.length) return;
    i = Math.max(0, Math.min(rows.length - 1, i));
    const fr = rows[i];
    activeKey = keyOf(fr);
    lastClickedIndex = i;
    // the current book follows (details pane); a group header only takes the cursor, and on
    // phones picking a book opens its page, so there the cursor moves alone
    if (!isMobile) {
      if (fr.kind === 'row') onPick(fr.row.book.id);
      else if (fr.kind === 'edition') onPick(fr.ed.id);
    }
    listRef?.reveal(i);
    mobileListRef?.reveal(i);
  }
  function openRow(fr: FlatRow) {
    if (fr.kind === 'group') { toggleCollapse(fr.group.key); return; }
    const id = fr.kind === 'row' ? fr.row.book.id : fr.ed.id;
    if (!isMobile) setDetailsCollapsed(false);
    onPick(id);
  }
  function pageRows(): number {
    const h = (isMobile ? mobileWrap : tableScroll)?.clientHeight ?? 400;
    return Math.max(1, Math.floor(h / (isMobile ? 64 : ROW_H)) - 1);
  }
  function revealActive() {
    tick().then(() => { if (activeIndex >= 0) { listRef?.reveal(activeIndex); mobileListRef?.reveal(activeIndex); } });
  }

  function tableKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement;
    // typing in the find box, the sort select and the column splitters keep their keys
    if (target.closest('input:not([type=checkbox]), select, [role=separator]')) return;
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const rows = flatRows;
    const cur = activeIndex;
    const fr = cur >= 0 ? rows[cur] : undefined;
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); moveTo(cur < 0 ? 0 : cur + 1); return;
      case 'ArrowUp': e.preventDefault(); moveTo(cur < 0 ? 0 : cur - 1); return;
      case 'Home': e.preventDefault(); moveTo(0); return;
      case 'End': e.preventDefault(); moveTo(rows.length - 1); return;
      case 'PageDown': e.preventDefault(); moveTo(Math.max(0, cur) + pageRows()); return;
      case 'PageUp': e.preventDefault(); moveTo(Math.max(0, cur) - pageRows()); return;
    }
    if (!fr) return;
    // a focused checkbox, link or button (other than a title) handles Space / Enter itself
    const own = target.closest('input[type=checkbox], a, button:not(.title-btn)');
    if (e.key === 'Enter') {
      if (own) return;
      e.preventDefault();
      openRow(fr);
    } else if (e.key === ' ') {
      if (own) return;
      e.preventDefault();
      if (fr.kind === 'row') toggle(lib, fr.row.book.id);
      else if (fr.kind === 'edition') toggle(lib, fr.ed.id);
      else toggleGroup(fr.group.rows.map((r) => r.book.id));
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      const open = e.key === 'ArrowRight';
      e.preventDefault();
      if (fr.kind === 'group') {
        const isOpen = !effectiveCollapsed.has(fr.group.key);
        if (open !== isOpen) toggleCollapse(fr.group.key);
        else if (open && rows[cur + 1]) moveTo(cur + 1);
      } else if (fr.kind === 'row') {
        const id = fr.row.book.id;
        if (open) {
          if (fr.row.book.editions && !openEditions.has(id)) toggleEditions(id);
        } else if (openEditions.has(id)) {
          toggleEditions(id);
        } else if (showGroupHeads) {
          // up to the series header, folded
          setCollapsed(fr.groupKey, true);
          activeKey = `g:${fr.groupKey}`;
          revealActive();
        }
      } else if (!open) {
        // an edition: back to its work, the editions folded
        const parent = fr.of;
        toggleEditions(parent);
        activeKey = `r:${parent}`;
        if (!isMobile) onPick(parent);
        revealActive();
      }
    }
  }

  /** A cover in the grid: the current book with its details shown, like a table row (the
   *  details pane reopens if it was folded away; on phones onPick opens the book page). */
  function pickCard(b: Book) {
    if (!isMobile) setDetailsCollapsed(false);
    activeKey = `r:${b.id}`;
    onPick(b.id);
  }

  // ---- columns ---------------------------------------------------------------------
  let liveCols = $state<Partial<Record<ColKey, number>>>({});
  const showNum = $derived(fullScope);
  type Col = ColKey | 'title';
  /** the rating column a rating sort is about: shown while sorting by it */
  const sortCol = $derived<OptCol | null>(
    sort === 'myRating' ? 'rating' : sort === 'libRating' ? 'libRating' : sort === 'extRating' ? 'extRating' : null,
  );
  /** columns asked for (the Columns menu, defaults, the sorted-by rating — that one right
   *  after the title, so a rating sort always shows its column without scrolling sideways) */
  function wantedIn(set: Set<string>): OptCol[] {
    const cols = OPT_COLUMNS.filter((k) => shownIn(set, k) && k !== sortCol);
    return sortCol ? [sortCol, ...cols] : cols;
  }
  const wantedCols = $derived(wantedIn(extraColumns));
  /** below this width the table scrolls sideways instead of squeezing the title */
  const w = (k: ColKey) => liveCols[k] ?? colWidth(k);
  const TITLE_MIN = 220;
  const widthOf = (cols: Col[]) =>
    32 + TITLE_MIN + cols.filter((c) => c !== 'title').reduce((n, c) => n + w(c as ColKey), 0) + 8 * cols.length + 28;
  /** The books pane (its live width: the details pane opening or closing, the window, the
   *  splitters) drops the columns it has no room for, least useful first, so the title keeps
   *  its minimum width. The sorted-by column stays, and so does a column the user asked for
   *  while it had no room (`+key`: a wide table scrolls sideways, checkbox and title fixed). */
  const DROP_ORDER: OptCol[] = ['language', 'format', 'genre', 'size', 'added', 'extRating', 'libRating', 'rating', 'series', 'author'];
  function hiddenIn(set: Set<string>, width: number): Set<OptCol> {
    const hidden = new Set<OptCol>();
    if (!width) return hidden;
    const room = width - 16; // the rows' scrollbar gutter
    const wanted = wantedIn(set);
    const base: Col[] = showNum ? ['num', 'title'] : ['title'];
    const shown = () => [...base, ...wanted.filter((c) => !hidden.has(c))];
    for (const c of DROP_ORDER) {
      if (widthOf(shown()) <= room) break;
      if (c === sortCol || set.has(`+${c}`) || !wanted.includes(c)) continue;
      hidden.add(c);
    }
    return hidden;
  }
  const autoHidden = $derived(hiddenIn(extraColumns, tableWidth));
  const columns = $derived.by<Col[]>(() => {
    const c: Col[] = [];
    if (showNum) c.push('num');
    c.push('title');
    for (const k of wantedCols) if (!autoHidden.has(k)) c.push(k);
    return c;
  });
  const titleIndex = $derived(columns.indexOf('title'));
  const tableMinWidth = $derived(widthOf(columns));
  /** left offsets of the sticky checkbox, # and Title cells (row padding 16, gaps 8) */
  const stickyNum = 16 + 32 + 8;
  const stickyTitle = $derived(showNum ? stickyNum + w('num') + 8 : stickyNum);
  const gridColumns = $derived(
    ['32px', ...columns.map((c) => (c === 'title' ? `minmax(${TITLE_MIN}px, 1fr)` : `${w(c)}px`))].join(' '),
  );
  const colLabel: Record<Col, string> = $derived({
    num: '#', title: t('books.col.title'), author: t('books.col.author'), series: t('books.col.series'),
    genre: t('books.col.genre'), language: t('books.col.language'), format: t('books.col.format'),
    size: t('books.col.size'), added: t('books.col.added'), rating: t('books.col.rating'),
    libRating: t('books.colShort.libRating'), extRating: t('books.col.extRating'),
  });

  function toggleColumn(c: OptCol) {
    const s = new Set(extraColumns);
    const on = colShown(c) && !autoHidden.has(c);
    const noRoom = autoHidden.has(c);
    s.delete(c); s.delete(`!${c}`); s.delete(`+${c}`);
    if (on) {
      if (c === 'size' || c === 'added') s.add(`!${c}`);
    } else {
      s.add(c);
      // Asked for while there is no room for it (or only by dropping a column on screen now):
      // it and the columns on screen stay, and the table scrolls sideways.
      const onScreen = wantedCols.filter((k) => !autoHidden.has(k) && k !== sortCol);
      const hid = hiddenIn(s, tableWidth);
      if (noRoom || hid.has(c) || onScreen.some((k) => hid.has(k))) {
        for (const k of [...onScreen, c]) { s.delete(`!${k}`); s.add(k); s.add(`+${k}`); }
      }
    }
    setPref(colsPrefKey, [...s]);
  }

  // resized / toggled columns change the table width: the rows' scroll position may be
  // clamped, and the header must follow it
  $effect(() => {
    gridColumns; tableMinWidth;
    tick().then(() => { if (headWrap && listRef) headWrap.scrollLeft = listRef.scrollLeft(); });
  });

  // a new sort, filter or scope starts at the left edge of a wide table
  $effect(() => {
    sort; ratingFilters; q; langFilter; extFilter; showDeleted; scopeKey;
    untrack(() => listRef?.resetX());
  });

  // The desktop table's fixed columns don't fit a phone screen (see
  // Phone.dc.html), so below 900px we switch to a simple stacked list.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 900px)');
    isMobile = mq.matches;
    const onChange = () => (isMobile = mq.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  });

  function rowMeta(r: Row): string {
    const b = r.book;
    const parts: string[] = [];
    if (scope.kind !== 'author') parts.push(authorsShort(b));
    if (b.series && !grouped && scope.kind !== 'series') parts.push(`${b.series.name}${b.serno ? ` #${b.serno}` : ''}`);
    else if (b.serno && (grouped || scope.kind === 'series')) parts.push(`#${b.serno}`);
    parts.push(b.ext.toUpperCase(), formatSize(b.size));
    if (b.libRating) parts.push(`★${b.libRating}`);
    if (b.extRating) parts.push(`OL ${formatAvg(b.extRating.avg)}`);
    return parts.filter(Boolean).join(' · ');
  }
  function authorsShort(b: Book): string {
    if (b.authors.length <= 2) return b.authors.map((a) => a.name).join(', ');
    return `${b.authors[0].name} ${t('books.andMore', { count: b.authors.length - 1 })}`;
  }

  // Cover grid: pages in more books when the end comes into view.
  function sentinel(node: HTMLElement) {
    const io = new IntersectionObserver((es) => { if (es.some((e) => e.isIntersecting)) loadMore(); });
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  // series of the selected books (loaded ones), for "Send whole series"
  const selectedSeries = $derived.by(() => {
    const sel = new Set(selectedIds(lib));
    const ids: number[] = [];
    for (const b of books) if (sel.has(b.id) && b.series && !ids.includes(b.series.id)) ids.push(b.series.id);
    return ids.slice(0, 20);
  });
  const shownCoauthors = $derived((header?.coauthors ?? []).slice(0, 3));
  const moreCoauthors = $derived(Math.max(0, (header?.coauthorCount ?? 0) - shownCoauthors.length));
  const sortOptions = $derived<SortKey[]>(
    scope.kind === 'author' ? ['series', 'title', 'date', 'myRating', 'libRating', 'extRating']
      : scope.kind === 'series' ? ['number', 'title', 'date', 'myRating', 'libRating', 'extRating']
        : ['date', 'myRating', 'libRating', 'extRating'],
  );
  const countLabel = $derived(
    fullScope && (q || hideAnth || langFilter || extFilter || ratingFilterCount(ratingFilters))
      ? t('books.shownOf', { shown: visibleBooks.length, total: books.length })
      : '',
  );
</script>

{#snippet filterMenu(phone: boolean)}
  <div class="col-menu" class:phone-menu={phone} role="menu" aria-label={t('books.filter')} data-testid="filter-menu"
    use:dismissable={{ onClose: () => (filterMenuOpen = false), trigger: () => filterBtn }} use:popover={{ anchor: () => filterBtn, placement: phone ? 'bottom-end' : 'bottom-start' }}>
    <label class="menu-check"><input type="checkbox" bind:checked={showDeleted} />{t('books.showDeleted')}</label>
    <label class="menu-check" title={t('editions.groupHint')}><input type="checkbox" data-testid="group-editions" checked={groupEditions} onchange={() => setPref('groupEditions', !groupEditions)} />{t('editions.group')}</label>
    {#if availableLangs.length > 1 || langFilter}
      <div class="menu-group-title">{t('search.language')}</div>
      <label><input type="radio" name="langf-{phone}" checked={langFilter === null} onchange={() => (langFilter = null)} />{t('books.any')}</label>
      {#each availableLangs as l (l)}
        <label><input type="radio" name="langf-{phone}" checked={langFilter === l} onchange={() => (langFilter = l)} />{l}</label>
      {/each}
    {/if}
    {#if availableExts.length > 1 || extFilter}
      <div class="menu-group-title">{t('search.format')}</div>
      <label><input type="radio" name="extf-{phone}" checked={extFilter === null} onchange={() => (extFilter = null)} />{t('books.any')}</label>
      {#each availableExts as e (e)}
        <label><input type="radio" name="extf-{phone}" checked={extFilter === e} onchange={() => (extFilter = e)} />{e.toUpperCase()}</label>
      {/each}
    {/if}
    <div class="menu-group-title">{t('ratings.filters')}</div>
    <RatingFilters filters={ratingFilters} onChange={(f) => (ratingFilters = f)} />
  </div>
{/snippet}

{#snippet cell(c: Col, b: Book, num: number | string)}
  {#if c === 'num'}<span class="muted num sticky-num">{num}</span>
  {:else if c === 'author'}<span class="muted ellipsis" title={b.authors.map((a) => a.name).join(', ')}>{authorsShort(b)}</span>
  {:else if c === 'series'}<span class="muted ellipsis" title={b.series?.name ?? ''}>{b.series ? `${b.series.name}${b.serno ? ` #${b.serno}` : ''}` : ''}</span>
  {:else if c === 'genre'}<span class="muted ellipsis">{genreNames.get(b.genres[0]) ?? ''}</span>
  {:else if c === 'language'}<span class="muted">{b.lang}</span>
  {:else if c === 'format'}<span class="muted">{b.ext}</span>
  {:else if c === 'size'}<span class="muted right">{formatSize(b.size)}</span>
  {:else if c === 'added'}<span class="muted right">{formatDate(b.date, i18nState.lang)}</span>
  {:else if c === 'rating'}<span class="right rating-cell" title={t('ratings.myTooltip')}>{#if b.rating}<Rating value={b.rating} size={11} />{/if}</span>
  {:else if c === 'libRating'}<span class="right rating-cell lib" title={b.libRating ? t('ratings.libTooltip', { n: b.libRating }) : t('ratings.libNone')}>{#if b.libRating}<span class="lib-num"><Icon name="star" size={12} strokeWidth={1.6} />{b.libRating}</span>{/if}</span>
  {:else if c === 'extRating'}<span class="right rating-cell">{#if b.extRating}<ExtRating value={b.extRating} />{/if}</span>
  {/if}
{/snippet}

{#snippet card(b: Book)}
  <!-- a click on the card (cover, title) makes it the current book and shows its details, like
       a table row; the checkbox (a sibling, not nested in the button) ticks it for batch -->
  <div class="cover-card" class:selected={b.id === selectedBookId} class:checked={isSelected(lib, b.id)} data-testid="cover-card">
    <button type="button" class="card-open" aria-current={b.id === selectedBookId ? 'true' : undefined} onclick={() => pickCard(b)} title={b.title}>
      <span class="cover-wrap">
        <CoverThumb {lib} bookId={b.id} title={b.title} width={140} height={200} />
      </span>
      <span class="cover-title">{#if grouped && b.serno}<span class="cover-no">#{b.serno}</span> {/if}{b.title}</span>
      {#if b.kidsAge !== null && b.kidsAge !== undefined || b.extRating || b.editions}
        <span class="cover-rate">
          {#if b.editions}<span class="tag ed-count" title={t('editions.toggleHint')}>{tn('editions.count', b.editions.count)}</span>{/if}
          <KidsBadge age={b.kidsAge} />{#if b.extRating}<ExtRating value={b.extRating} />{/if}
        </span>
      {/if}
      {#if scope.kind !== 'author'}<span class="cover-sub">{authorsShort(b)}</span>{/if}
    </button>
    {#if isSelected(lib, b.id)}
      <span class="check-badge" aria-hidden="true"><Icon name="check" size={14} /></span>
    {/if}
    <input
      type="checkbox"
      class="grid-check"
      aria-label={t('books.select', { title: b.title })}
      checked={isSelected(lib, b.id)}
      onchange={() => toggle(lib, b.id)}
    />
  </div>
{/snippet}

{#snippet colHead(c: Col, i: number)}
  <span class="hcell" class:sticky-num={c === 'num'} class:sticky-title={c === 'title'} class:right={c === 'size' || c === 'added' || c === 'rating' || c === 'libRating' || c === 'extRating'}>
    <span class="hlabel" title={c === 'libRating' ? t('books.col.libRating') : undefined}>{colLabel[c]}</span>
    {#if c !== 'title'}
      {@const k = c as ColKey}
      {#if i < titleIndex}
        <Splitter
          class="col-split at-right"
          target="none"
          value={w(k)} min={COL_LIMITS[k].min} max={COL_LIMITS[k].max}
          label={t('layout.resizeColumn', { name: colLabel[c] })}
          side="before"
          onInput={(v) => (liveCols = { ...liveCols, [k]: v })}
          onCommit={(v) => { setColWidth(k, v); liveCols = {}; }}
          onReset={() => { setColWidth(k, null); liveCols = {}; }}
        />
      {:else}
        <Splitter
          class="col-split at-left"
          target="none"
          value={w(k)} min={COL_LIMITS[k].min} max={COL_LIMITS[k].max}
          label={t('layout.resizeColumn', { name: colLabel[c] })}
          side="after"
          onInput={(v) => (liveCols = { ...liveCols, [k]: v })}
          onCommit={(v) => { setColWidth(k, v); liveCols = {}; }}
          onReset={() => { setColWidth(k, null); liveCols = {}; }}
        />
      {/if}
    {/if}
  </span>
{/snippet}

<main class="books-pane" aria-busy={loading}>
  {#if header && !isMobile}
    <div class="scope-header">
      <div class="crumb">{header.crumb}</div>
      <div class="h1-row">
        <h1 title={header.name}>{header.name}</h1>
        {#if header.follow}<FollowButton {lib} kind={header.follow.kind} id={header.follow.id} />{/if}
      </div>
      <div class="counts">
        <span data-testid="header-count" title={t('browse.countsHint')}>{tn('browse.booksCount', header.booksCount)}</span>
        {#if header.filesCount && header.filesCount !== header.booksCount}<span data-testid="header-files">· {tn('browse.filesCount', header.filesCount)}</span>{/if}
        {#if header.seriesCount}<span>· {tn('browse.seriesCount', header.seriesCount)}</span>{/if}
        {#if header.anthologies}<span>· {tn('browse.inAnthologies', header.anthologies)}</span>{/if}
        {#if scope.kind === 'series' && onSendSeries}
          <button type="button" class="series-send" data-testid="send-series" onclick={() => onSendSeries!([scope.id])}><Icon name="send" size={14} />{t('series.sendWhole')}</button>
        {/if}
      </div>
      {#if shownCoauthors.length || moreCoauthors}
        <div class="coauthors">
          <span class="lbl">{t('authors.alsoWith')}</span>
          <span class="names">
            {#each shownCoauthors as a, i (a.id)}{#if i > 0}{', '}{/if}<a href="/l/{lib}/authors/{a.id}" data-link title={tn('authors.sharedBooks', a.books)}>{a.name}</a>{/each}
          </span>
          {#if moreCoauthors}
            <div class="more-wrap">
              <button type="button" class="more-btn" bind:this={coauthorsBtn} aria-haspopup="dialog" aria-expanded={coauthorsOpen} onclick={() => (coauthorsOpen = !coauthorsOpen)}>
                {shownCoauthors.length ? t('authors.andMore', { count: moreCoauthors }) : tn('authors.coauthorsCount', moreCoauthors)}
              </button>
              {#if coauthorsOpen && scope.kind === 'author'}
                <CoauthorsPopover {lib} authorId={scope.id} trigger={coauthorsBtn} onClose={() => (coauthorsOpen = false)} />
              {/if}
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {:else if header && isMobile}
    <div class="scope-header-phone">
      <button type="button" class="back" aria-label={t('common.back')} onclick={onBack}><Icon name="chevronLeft" size={18} /></button>
      <div class="ph-info">
        <span class="ph-name">{header.name}</span>
        <span class="ph-counts">
          {tn('browse.booksCount', header.booksCount)}{#if header.seriesCount}&nbsp;· {tn('browse.seriesCount', header.seriesCount)}{/if}
        </span>
      </div>
      {#if header.follow}<FollowButton {lib} kind={header.follow.kind} id={header.follow.id} compact />{/if}
      {#if scope.kind === 'series' && onSendSeries}
        <button type="button" class="series-send" aria-label={t('series.sendWhole')} title={t('series.sendWhole')} onclick={() => onSendSeries!([scope.id])}><Icon name="send" size={16} /></button>
      {/if}
    </div>
  {/if}

  <div class="toolbar">
    <label class="find">
      <Icon name="search" size={15} />
      <input
        type="search"
        placeholder={scope.kind === 'author' ? t('books.findInAuthor') : t('books.findInList')}
        aria-label={t('books.findInList')}
        bind:value={text}
        onkeydown={(e) => { if (e.key === 'Escape') text = ''; }}
      />
    </label>
    <label class="sort">
      <span class="visually-hidden">{t('books.sort')}</span>
      <select data-testid="books-sort" value={sort} onchange={(e) => setPref(sortPrefKey, (e.currentTarget as HTMLSelectElement).value)} aria-label={t('books.sort')}>
        {#each sortOptions as o (o)}<option value={o}>{t(`books.sort.${o}`)}</option>{/each}
      </select>
    </label>
    {#if groups.length > 3 && !q}
      <button type="button" class="tbtn" data-testid="expand-all" onclick={() => setAllCollapsed(allExpanded)}>
        {allExpanded ? t('books.collapseAll') : t('books.expandAll')}
      </button>
    {/if}
    {#if scope.kind === 'author' && (anthCount > 0 || hideAnth)}
      <button type="button" class="tbtn toggle" aria-pressed={hideAnth} class:on={hideAnth}
        title={t('books.anthologiesHint', { n: ANTHOLOGY_MIN_AUTHORS })}
        onclick={() => setPref('hideAnthologies', !hideAnth)}>
        {hideAnth ? t('books.anthologiesHidden', { count: anthCount }) : t('books.hideAnthologies', { count: anthCount })}
      </button>
    {/if}
    <div class="filter-chooser">
      <button type="button" class="tbtn" bind:this={filterBtn} class:on={activeFilterCount > 0} aria-haspopup="true" aria-expanded={filterMenuOpen} onclick={() => (filterMenuOpen = !filterMenuOpen)}>
        <Icon name="filter" size={14} /><span class="lbl-f">{t('books.filter')}</span>{#if activeFilterCount}<span class="badge">{activeFilterCount}</span>{/if}
      </button>
      {#if filterMenuOpen}{@render filterMenu(isMobile)}{/if}
    </div>
    {#if countLabel}<span class="shown">{countLabel}</span>{/if}
    <div class="spacer"></div>
    {#if !isMobile}
      <div class="col-chooser">
        <button type="button" class="tbtn" bind:this={columnBtn} onclick={() => (columnMenuOpen = !columnMenuOpen)} aria-haspopup="true" aria-expanded={columnMenuOpen}>
          {t('books.columns')}<Icon name="chevronDown" size={14} />
        </button>
        {#if columnMenuOpen}
          <div class="col-menu" role="menu" aria-label={t('books.columns')} data-testid="columns-menu"
            use:dismissable={{ onClose: () => (columnMenuOpen = false), trigger: () => columnBtn }} use:popover={{ anchor: () => columnBtn, placement: 'bottom-end' }}>
            {#each OPT_COLUMNS as c (c)}
              <label class:dim={autoHidden.has(c)} title={autoHidden.has(c) ? t('books.colNoRoom') : c === sortCol && !colShown(c) ? t('books.colSorted') : undefined}>
                <input type="checkbox" checked={(colShown(c) && !autoHidden.has(c)) || c === sortCol} disabled={c === sortCol} onchange={() => toggleColumn(c)} />
                {t(`books.col.${c}`)}
                {#if autoHidden.has(c)}<span class="col-note">{t('books.colNoRoomShort')}</span>{:else if c === sortCol}<span class="col-note">{t('books.colSortedShort')}</span>{/if}
              </label>
            {/each}
            {#if autoHidden.size}<p class="col-hidden-note" data-testid="columns-hidden">{tn('books.colsHidden', autoHidden.size)}</p>{/if}
          </div>
        {/if}
      </div>
      <div role="group" aria-label={t('books.view')} class="view-toggle">
        <button type="button" aria-label={t('books.viewTable')} aria-pressed={view === 'table'} class:active={view === 'table'} onclick={() => setPref('booksView', 'table')}>
          <Icon name="table" size={16} />
        </button>
        <button type="button" aria-label={t('books.viewGrid')} aria-pressed={view === 'grid'} class:active={view === 'grid'} onclick={() => setPref('booksView', 'grid')}>
          <Icon name="grid" size={16} />
        </button>
      </div>
    {/if}
  </div>

  {#if loading && books.length === 0}
    <div class="skeleton" aria-label={t('common.loading')}>
      {#each Array(8) as _, i (i)}<div class="sk-row"><span></span><span style="width: {40 + ((i * 37) % 45)}%"></span></div>{/each}
    </div>
  {:else if loadError}
    <div class="empty">
      <p>{t('common.error')}: {loadError}</p>
      <button type="button" class="tbtn" onclick={() => reloadTick++}>{t('common.retry')}</button>
    </div>
  {:else if visibleBooks.length === 0}
    <div class="empty">
      {#if anyFilter}
        <p>{t('books.nothingMatches')}</p>
        <button type="button" class="tbtn" onclick={resetFilters}>{t('books.resetFilters')}</button>
      {:else}
        <p>{t('books.noBooks')}</p>
      {/if}
    </div>
  {:else if isMobile}
    <!-- keyboard (↑/↓, Enter, Space, ←/→): tableKeydown, as in the table -->
    <div class="mobile-list" bind:this={mobileWrap} role="grid" tabindex="-1" aria-label={t('books.list')}
      aria-activedescendant={activeIndex >= 0 ? rowDomId(activeIndex) : undefined} onkeydown={tableKeydown}>
      <VirtualList bind:this={mobileListRef} items={flatRows} itemHeight={64} overscan={6} onRangeChange={onRowRangeChange}>
        {#snippet row(fr, fi)}
          {#if fr.kind === 'group'}
            <button type="button" class="m-group" id={rowDomId(fi)} class:cursor={fi === activeIndex} aria-expanded={!effectiveCollapsed.has(fr.group.key)} onclick={() => { activeKey = keyOf(fr); toggleCollapse(fr.group.key); }}>
              <span class="chev" class:open={!effectiveCollapsed.has(fr.group.key)}><Icon name="chevronRight" size={14} /></span>
              <span class="gname">{fr.group.name}</span><span class="gcount">{fr.group.count}</span>
            </button>
          {:else if fr.kind === 'edition'}
            {@const e = fr.ed}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div class="m-row m-edition" role="row" tabindex="-1" id={rowDomId(fi)} data-testid="edition-row" class:checked={isSelected(lib, e.id)} class:cursor={fi === activeIndex}
              onclick={() => { activeKey = keyOf(fr); onPick(e.id); }}>
              <span class="ed-mark"><Icon name="layers" size={14} /></span>
              <div class="m-info">
                {#if differentTitle(e, fr.ofTitle)}<span class="m-ed-title" data-testid="edition-title">{e.title}</span>{/if}
                <span class="m-meta">{editionLine(e)}</span>
              </div>
              <input type="checkbox" aria-label={t('books.select', { title: e.title })} checked={isSelected(lib, e.id)}
                onclick={(ev) => ev.stopPropagation()} onchange={() => toggle(lib, e.id)} />
            </div>
          {:else}
            {@const r = fr.row}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="m-row"
              role="row"
              tabindex={fi === Math.max(0, activeIndex) ? 0 : -1}
              id={rowDomId(fi)}
              class:checked={isSelected(lib, r.book.id)}
              class:deleted={r.book.deleted}
              class:cursor={fi === activeIndex}
              onclick={() => { activeKey = keyOf(fr); onPick(r.book.id); }}
            >
              <CoverThumb {lib} bookId={r.book.id} title={r.book.title} width={36} height={52} />
              <div class="m-info">
                <span class="m-title-row"><span class="m-title">{r.book.title}</span><KidsBadge age={r.book.kidsAge} /></span>
                <span class="m-meta">{#if r.book.editions}<button type="button" class="ed-tag" data-testid="editions-toggle" aria-expanded={openEditions.has(r.book.id)} onclick={(ev) => { ev.stopPropagation(); toggleEditions(r.book.id); }}>{tn('editions.count', r.book.editions.count)}</button> {/if}{rowMeta(r)}</span>
              </div>
              <input
                type="checkbox"
                aria-label={t('books.select', { title: r.book.title })}
                checked={isSelected(lib, r.book.id)}
                onclick={(e) => e.stopPropagation()}
                onchange={() => toggle(lib, r.book.id)}
              />
            </div>
          {/if}
        {/snippet}
      </VirtualList>
    </div>
  {:else if view === 'table'}
    <div class="table-wrap" bind:this={tableWrap} class:scrolled-x={scrolledX} style="--sticky-num: {stickyNum}px; --sticky-title: {stickyTitle}px">
      <div class="head-wrap" bind:this={headWrap}>
        <div class="brow head" role="row" style="grid-template-columns: {gridColumns}; min-width: {tableMinWidth}px">
          <span class="cell-check"><input type="checkbox" aria-label={t('books.selectAll')} checked={allChecked} onchange={toggleAll} /></span>
          {#each columns as c, i (c)}{@render colHead(c, i)}{/each}
        </div>
      </div>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div class="scroll" bind:this={tableScroll} tabindex="0" role="grid" aria-label={t('books.list')} data-testid="books-grid"
        aria-activedescendant={activeIndex >= 0 ? rowDomId(activeIndex) : undefined} onkeydown={tableKeydown}>
        <VirtualList bind:this={listRef} items={flatRows} itemHeight={ROW_H} onRangeChange={onRowRangeChange} scrollX {onScrollX}>
          {#snippet row(fr, fi)}
            {#if fr.kind === 'group'}
              {@const open = !effectiveCollapsed.has(fr.group.key)}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div class="group-head" role="row" tabindex="-1" id={rowDomId(fi)} class:cursor={fi === activeIndex} aria-expanded={open} data-testid="group-row" onclick={() => (activeKey = keyOf(fr))}>
                <input
                  type="checkbox"
                  aria-label={t('books.selectGroup', { name: fr.group.name })}
                  checked={fr.group.rows.every((r) => isSelected(lib, r.book.id))}
                  onchange={() => toggleGroup(fr.group.rows.map((r) => r.book.id))}
                />
                <button type="button" class="gtoggle" aria-expanded={open} onclick={() => toggleCollapse(fr.group.key)}>
                  <span class="chev" class:open><Icon name="chevronRight" size={14} /></span>
                  <span class="gname">{fr.group.name}</span>
                  <span class="gcount">{fr.group.count}</span>
                </button>
                {#if fr.group.seriesId !== null}
                  <a class="glink" href="/l/{lib}/series/{fr.group.seriesId}" data-link title={t('books.openSeries')} aria-label={t('books.openSeries')}><Icon name="external" size={13} /></a>
                {/if}
              </div>
            {:else if fr.kind === 'edition'}
              {@const e = fr.ed}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div class="brow edition-row" role="row" tabindex="-1" data-testid="edition-row" id={rowDomId(fi)}
                class:selected={e.id === selectedBookId} class:checked={isSelected(lib, e.id)} class:deleted={e.deleted} class:cursor={fi === activeIndex}
                style="grid-template-columns: {gridColumns}; min-width: {tableMinWidth}px" onclick={() => { activeKey = keyOf(fr); lastClickedIndex = fi; onPick(e.id); }}>
                <span class="cell-check"><input type="checkbox" aria-label={t('books.select', { title: e.title })} checked={isSelected(lib, e.id)}
                  onclick={(ev) => ev.stopPropagation()} onchange={() => toggle(lib, e.id)} /></span>
                {#each columns as c (c)}
                  {#if c === 'title'}
                    <span class="title-cell sticky-title ed-cell" title={`${e.title} — ${editionLine(e)}`}>
                      <span class="ed-icon"><Icon name="layers" size={13} /></span>
                      {#if differentTitle(e, fr.ofTitle)}<span class="ed-title" data-testid="edition-title">{e.title}</span>{/if}
                      <span class="ed-meta">{editionLine(e)}</span>
                    </span>
                  {:else}
                    {@render cell(c, e, '')}
                  {/if}
                {/each}
              </div>
            {:else}
              {@const r = fr.row}
              {@const b = r.book}
              <!-- keyboard: the grid container handles ↑/↓/Space (tableKeydown) -->
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div
                class="brow"
                role="row"
                tabindex="-1"
                id={rowDomId(fi)}
                data-testid="book-row"
                aria-selected={b.id === selectedBookId}
                class:cursor={fi === activeIndex}
                class:selected={b.id === selectedBookId}
                class:checked={isSelected(lib, b.id)}
                class:deleted={b.deleted}
                style="grid-template-columns: {gridColumns}; min-width: {tableMinWidth}px"
                onclick={(e) => rowClick(e, b, fi)}
              >
                <span class="cell-check"><input
                  type="checkbox"
                  aria-label={t('books.select', { title: b.title })}
                  checked={isSelected(lib, b.id)}
                  onclick={(e) => e.stopPropagation()}
                  onchange={() => toggle(lib, b.id)}
                /></span>
                {#each columns as c (c)}
                  {#if c === 'num'}<span class="muted num sticky-num">{r.num}</span>
                  {:else if c === 'title'}
                    <span class="title-cell sticky-title" title={b.title}>
                      <button type="button" class="title-btn" tabindex="-1" class:strong={b.id === selectedBookId}>{b.title}</button>
                      {#if scope.kind === 'author' && !grouped && b.series && !extraColumns.has('series')}<span class="sub">{b.series.name}{b.serno ? ` #${b.serno}` : ''}</span>{/if}
                      {#if b.editions}<button type="button" class="tag ed-tag" data-testid="editions-toggle" tabindex="-1" aria-expanded={openEditions.has(b.id)} title={t('editions.toggleHint')} onclick={(ev) => { ev.stopPropagation(); toggleEditions(b.id); }}>{tn('editions.count', b.editions.count)}</button>{/if}
                      {#if isAnthology(b)}<span class="tag" title={b.authors.map((a) => a.name).join(', ')}>{tn('books.authorsCount', b.authors.length)}</span>{/if}
                      <KidsBadge age={b.kidsAge} />
                      {#if b.deleted}<span class="tag danger">{t('books.deleted')}</span>{/if}
                    </span>
                  {:else}{@render cell(c, b, r.num)}
                  {/if}
                {/each}
              </div>
            {/if}
          {/snippet}
        </VirtualList>
        {#if fetchingMore}<div class="loading-more">{t('common.loading')}</div>{/if}
      </div>
    </div>
  {:else}
    <div class="grid-view">
      {#if showGroupHeads}
        <!-- grouped by series: the same groups (and collapsed state) as the table -->
        {#each groups as g (g.key)}
          {@const open = !effectiveCollapsed.has(g.key)}
          <div class="grid-group" role="heading" aria-level="3">
            <button type="button" class="gtoggle" aria-expanded={open} onclick={() => toggleCollapse(g.key)}>
              <span class="chev" class:open><Icon name="chevronRight" size={14} /></span>
              <span class="gname">{g.name}</span>
              <span class="gcount">{g.count}</span>
            </button>
          </div>
          {#if open}
            {#each g.rows as r (r.book.id)}{@render card(r.book)}{/each}
          {/if}
        {/each}
      {:else}
        {#each visibleBooks as b (b.id)}{@render card(b)}{/each}
      {/if}
      {#if nextCursor}<div class="grid-sentinel" use:sentinel>{fetchingMore ? t('common.loading') : ''}</div>{/if}
    </div>
  {/if}

  <SelectionBar
    count={selectedCount(lib)}
    onSendSeries={onSendSeries && selectedSeries.length ? () => onSendSeries!(selectedSeries) : undefined}
    onSend={() => onOpenSend(selectedIds(lib))}
    onDownload={() => onOpenSend(selectedIds(lib))}
    onShelf={() => onOpenShelf(selectedIds(lib))}
    onClear={() => clearSelection(lib)}
  />
</main>

<style>
  .books-pane { flex: 1 1 0; min-width: 320px; display: flex; flex-direction: column; background: var(--surface); position: relative; min-height: 0; }
  .scope-header { padding: 16px 24px 12px; display: flex; flex-direction: column; gap: 4px; border-bottom: 1px solid var(--line); flex-shrink: 0; min-width: 0; }
  .scope-header .crumb { font-size: 12px; color: var(--muted); }
  .series-send {
    display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 10px; margin-left: 10px; border-radius: 7px;
    border: 1px solid var(--accent); background: transparent; color: var(--accent); font-size: 13px; font-weight: 500; cursor: pointer;
  }
  .series-send:hover { background: var(--accent-soft); }
  .scope-header-phone .series-send { margin-left: auto; width: 36px; height: 36px; padding: 0; justify-content: center; }
  .scope-header h1 {
    margin: 2px 0 2px; font-family: var(--font-display); font-size: 24px; font-weight: 600; line-height: 1.25;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .h1-row { display: flex; align-items: center; gap: 12px; min-width: 0; }
  .h1-row h1 { min-width: 0; }
  .ed-tag { border: none; cursor: pointer; font: inherit; font-size: 11px; color: var(--accent-soft-ink); background: var(--accent-soft); border-radius: 4px; padding: 1px 6px; white-space: nowrap; }
  .ed-tag:hover { text-decoration: underline; }
  .ed-tag[aria-expanded='true'] { background: var(--accent); color: #fff; }
  /* an edition under its work: the same grid as a book row (the columns line up), a muted
     title cell with the edition's own title (when it differs) and its file facts */
  .brow.edition-row { --row-bg: var(--surface-alt); font-size: 13px; }
  .brow.edition-row:hover { --row-bg: var(--row-hover); }
  .brow.edition-row.checked { --row-bg: var(--row-checked); }
  .brow.edition-row.selected { --row-bg: var(--accent-soft); }
  .ed-cell { gap: 8px; padding-left: 14px; color: var(--muted-2); overflow: hidden; }
  .ed-icon { display: inline-flex; color: var(--muted); flex-shrink: 0; }
  .ed-title { color: var(--ink); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 0 1 auto; min-width: 0; }
  .ed-meta { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 0 10 auto; min-width: 0; }
  .m-edition { background: var(--surface-alt); }
  .m-ed-title { font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ed-mark { width: 36px; display: flex; justify-content: center; color: var(--muted); flex-shrink: 0; }
  .counts { display: flex; align-items: center; gap: 4px; font-size: 13px; color: var(--muted); white-space: nowrap; overflow: hidden; }
  .coauthors { display: flex; align-items: baseline; gap: 6px; font-size: 13px; color: var(--muted); min-width: 0; }
  .coauthors .lbl { flex-shrink: 0; }
  .coauthors .names { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .coauthors a { color: var(--muted-2); }
  .coauthors a:hover { color: var(--accent); }
  .more-wrap { position: relative; flex-shrink: 0; }
  .more-btn { border: none; background: none; padding: 0; color: var(--accent); font-size: 13px; white-space: nowrap; }
  .more-btn:hover { text-decoration: underline; }
  .scope-header-phone { display: flex; align-items: center; gap: 8px; padding: 10px 12px; border-bottom: 1px solid var(--line); flex-shrink: 0; position: relative; }
  .scope-header-phone .back { width: 36px; height: 36px; border: none; background: transparent; border-radius: 8px; display: flex; align-items: center; justify-content: center; flex-shrink: 0; }
  .scope-header-phone .back:hover { background: var(--surface-hover); }
  .ph-info { flex-grow: 1; min-width: 0; display: flex; flex-direction: column; }
  .ph-name { font-family: var(--font-display); font-size: 16px; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ph-counts { font-size: 12px; color: var(--muted); }
  .loading-more { text-align: center; padding: 10px; font-size: 12px; color: var(--muted); }
  .toolbar { padding: 10px 16px 10px 24px; border-bottom: 1px solid var(--line); flex-shrink: 0; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .find {
    display: flex; align-items: center; gap: 6px; height: 32px; padding: 0 8px; flex: 1 1 160px; max-width: 300px; min-width: 120px;
    border: 1px solid var(--border); border-radius: 6px; background: var(--surface); color: var(--muted);
  }
  .find:focus-within { border-color: var(--accent); }
  .find input { border: none; outline: none; background: transparent; flex-grow: 1; min-width: 0; font: inherit; font-size: 13px; color: var(--ink); }
  .sort select {
    height: 32px; padding: 0 6px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface);
    color: var(--muted-2); font: inherit; font-size: 13px;
  }
  .spacer { flex-grow: 1; }
  .shown { font-size: 12px; color: var(--muted); white-space: nowrap; }
  .tbtn {
    display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 10px; white-space: nowrap;
    border: 1px solid var(--border); border-radius: 6px; background: var(--surface); color: var(--muted-2); font-size: 13px;
  }
  .tbtn:hover { background: var(--surface-hover); }
  .tbtn.on { background: var(--accent-soft); color: var(--accent-soft-ink); border-color: var(--accent); }
  .badge { min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--accent); color: #fff; font-size: 11px; display: inline-flex; align-items: center; justify-content: center; }
  .filter-chooser, .col-chooser { position: relative; }
  .col-menu {
    position: absolute; top: 38px; left: 0; z-index: 20; background: var(--surface); border: 1px solid var(--line);
    border-radius: 8px; box-shadow: 0 8px 24px rgba(0,0,0,.15); padding: 8px; display: flex; flex-direction: column; gap: 2px; min-width: 180px;
    max-height: 60vh; overflow-y: auto;
  }
  .col-chooser .col-menu { left: auto; right: 0; }
  .col-menu label { display: flex; align-items: center; gap: 8px; font-size: 13px; padding: 4px 6px; border-radius: 4px; }
  .col-menu label:hover { background: var(--surface-hover); }
  .col-menu label.dim { color: var(--muted); }
  .col-hidden-note { margin: 4px 0 0; padding: 6px 8px 2px; border-top: 1px solid var(--line-soft); font-size: 12px; color: var(--muted); max-width: 220px; }
  .col-note { margin-left: auto; padding-left: 10px; font-size: 11px; color: var(--muted); white-space: nowrap; }
  /* the keyboard cursor (ARIA grid): a ring on the current row while the list has focus */
  .scroll:focus-within .cursor, .mobile-list:focus-within .cursor { box-shadow: inset 0 0 0 2px var(--focus); }
  .mobile-list { outline: none; }
  .menu-check { border-bottom: 1px solid var(--line-soft); padding-bottom: 8px !important; margin-bottom: 4px; }
  .menu-group-title { font-size: 11px; font-weight: 600; color: var(--muted); text-transform: uppercase; letter-spacing: .04em; padding: 6px 6px 2px; }
  .view-toggle { display: flex; border: 1px solid var(--border); border-radius: 6px; overflow: hidden; }
  .view-toggle button { width: 34px; height: 30px; border: none; background: var(--surface); color: var(--muted-2); display: flex; align-items: center; justify-content: center; }
  .view-toggle button + button { border-left: 1px solid var(--border); }
  .view-toggle button.active { background: var(--surface-hover); color: var(--ink); }
  .empty { flex-grow: 1; display: flex; flex-direction: column; gap: 10px; align-items: center; justify-content: center; color: var(--muted); font-size: 14px; padding: 24px; text-align: center; }
  .empty p { margin: 0; }
  .skeleton { padding: 8px 24px; display: flex; flex-direction: column; }
  .sk-row { display: flex; gap: 16px; align-items: center; height: 40px; border-bottom: 1px solid var(--line-soft); }
  .sk-row span { display: block; height: 10px; border-radius: 5px; background: var(--surface-hover); animation: pulse 1.2s ease-in-out infinite; }
  .sk-row span:first-child { width: 16px; }
  @keyframes pulse { 50% { opacity: .45; } }
  .mobile-list { flex-grow: 1; min-height: 0; position: relative; }
  .m-group {
    all: unset; box-sizing: border-box; width: 100%; height: 64px; padding: 0 16px; font-size: 13px; font-weight: 600; color: var(--muted-2);
    display: flex; align-items: center; gap: 8px; background: var(--surface-alt); border-bottom: 1px solid var(--line-soft); cursor: pointer;
  }
  .m-group .gname { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .m-group .gcount { font-weight: 400; color: var(--muted); margin-left: auto; }
  .m-row { display: flex; align-items: center; gap: 12px; padding: 6px 16px; height: 64px; box-sizing: border-box; }
  .m-row.checked { background: var(--accent-soft); }
  .m-info { flex-grow: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .m-title { font-size: 15px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .m-title-row { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .m-meta { font-size: 12px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .m-row input[type='checkbox'] { width: 22px; height: 22px; flex-shrink: 0; }
  .deleted .m-title, .deleted .title-btn { text-decoration: line-through; color: var(--muted); }
  @media (max-width: 900px) {
    .books-pane { min-width: 0; }
    .toolbar { padding: 8px 12px; }
    .find { max-width: none; flex-basis: 100%; }
    .toolbar .lbl-f, .toolbar .shown { display: none; }
    .col-menu.phone-menu { left: auto; right: 0; }
  }
  .table-wrap { flex-grow: 1; overflow: hidden; display: flex; flex-direction: column; min-height: 0; }
  .scroll { flex-grow: 1; min-height: 0; position: relative; display: flex; flex-direction: column; outline: none; }
  .scroll:focus-visible { box-shadow: inset 0 0 0 2px var(--focus); }
  /* the header and the rows reserve the same scrollbar gutter, so that the 1fr title column
     (and every column after it) has the same width in both with classic scrollbars */
  .scroll :global(.vlist) { flex-grow: 1; min-height: 0; scrollbar-gutter: stable; }
  .brow { --row-bg: var(--surface); background: var(--row-bg); display: grid; align-items: center; height: 40px; padding: 0 12px 0 16px; border-bottom: 1px solid var(--line-soft); font-size: 14px; cursor: default; gap: 8px; }
  .head-wrap { overflow: hidden; flex-shrink: 0; scrollbar-gutter: stable; }
  /* wide tables scroll sideways; checkbox, # and Title stay put (the cells cover the gaps) */
  .cell-check, .sticky-num, .sticky-title { position: sticky; z-index: 1; background: var(--row-bg); align-self: stretch; display: flex; align-items: center; }
  .cell-check { left: 16px; box-shadow: -16px 0 0 var(--row-bg), 8px 0 0 var(--row-bg); }
  .sticky-num { left: var(--sticky-num); box-shadow: 8px 0 0 var(--row-bg); }
  .sticky-title { left: var(--sticky-title); }
  .scrolled-x .sticky-title { box-shadow: 8px 0 0 var(--row-bg), 14px 0 10px -6px rgba(0, 0, 0, .22); }

  .brow.head { --row-bg: var(--surface-alt); height: 34px; font-size: 12px; font-weight: 600; color: var(--muted); background: var(--surface-alt); flex-shrink: 0; }
  .hcell { position: relative; min-width: 0; height: 100%; display: flex; align-items: center; }
  .hcell.right { justify-content: flex-end; }
  .hlabel { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .hcell :global(.splitter.col-split) { position: absolute; top: 4px; bottom: 4px; margin: 0; }
  /* centred on the middle of the 8px column gap */
  .hcell :global(.splitter.col-split.at-right) { left: calc(100% - .5px); }
  .hcell :global(.splitter.col-split.at-left) { left: -8.5px; }
  .hcell :global(.splitter.col-split)::after { opacity: .25; background: var(--border-dashed); }
  .hcell :global(.splitter.col-split:hover)::after { opacity: 1; background: var(--accent); }
  .brow:not(.head):hover { --row-bg: var(--row-hover); }
  .brow.checked { --row-bg: var(--row-checked); }
  .brow.selected { --row-bg: var(--accent-soft); }
  .group-head { position: sticky; left: 0; display: flex; align-items: center; gap: 8px; height: 40px; padding: 0 12px 0 16px; background: var(--page); border-bottom: 1px solid var(--line-soft); font-size: 13px; min-width: 0; }
  .gtoggle { all: unset; display: flex; align-items: center; gap: 6px; min-width: 0; flex: 0 1 auto; cursor: pointer; padding: 4px 4px; border-radius: 4px; }
  .gtoggle:hover { background: var(--surface-hover); }
  .gtoggle:focus-visible { outline: 2px solid var(--focus); }
  .chev { display: inline-flex; color: var(--muted); transition: transform .12s; flex-shrink: 0; }
  .chev.open { transform: rotate(90deg); }
  .gname { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .gcount { color: var(--muted); flex-shrink: 0; font-variant-numeric: tabular-nums; }
  .glink { display: inline-flex; color: var(--muted); padding: 4px; border-radius: 4px; }
  .glink:hover { color: var(--accent); background: var(--surface-hover); }
  .muted { color: var(--muted); }
  .num { font-variant-numeric: tabular-nums; }
  .right { text-align: right; justify-self: end; }
  .rating-cell { display: flex; justify-content: flex-end; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .title-cell { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
  .title-btn { all: unset; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; cursor: pointer; min-width: 0; flex: 0 1 auto; }
  .title-btn.strong { font-weight: 600; }
  .sub { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 0 10 auto; min-width: 0; }
  .tag { flex-shrink: 0; font-size: 11px; color: var(--muted-2); background: var(--surface-hover); border-radius: 4px; padding: 1px 6px; white-space: nowrap; }
  .tag.danger { color: var(--danger); }
  .grid-view { flex-grow: 1; overflow-y: auto; padding: 20px 24px 80px; display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 20px 16px; align-content: start; }
  .grid-group { grid-column: 1 / -1; display: flex; align-items: center; border-bottom: 1px solid var(--line-soft); padding: 2px 0 6px; margin-top: 4px; font-size: 13px; }
  .grid-group:first-child { margin-top: 0; }
  .cover-no { color: var(--muted); font-variant-numeric: tabular-nums; margin-right: .3em; }
  .ed-count { color: var(--accent-soft-ink); background: var(--accent-soft); }
  .grid-sentinel { grid-column: 1 / -1; height: 24px; text-align: center; font-size: 12px; color: var(--muted); }
  .cover-card { position: relative; min-width: 0; }
  .card-open { all: unset; box-sizing: border-box; width: 100%; display: flex; flex-direction: column; gap: 4px; cursor: pointer; min-width: 0; }
  .card-open:focus-visible { outline: 2px solid var(--focus); outline-offset: 4px; border-radius: 4px; }
  .cover-wrap { position: relative; margin-bottom: 4px; display: block; }
  .cover-wrap :global(.cover) { width: 100% !important; aspect-ratio: 2/3; height: auto !important; }
  .cover-card.selected .cover-wrap :global(.cover) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .check-badge { position: absolute; top: 8px; right: 8px; pointer-events: none; width: 22px; height: 22px; border-radius: 11px; background: #FFF; display: flex; align-items: center; justify-content: center; color: var(--accent); }
  .grid-check { position: absolute; top: 8px; left: 8px; width: 16px; height: 16px; }
  .cover-title { font-size: 13px; line-height: 1.3; overflow: hidden; text-overflow: ellipsis; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; }
  .cover-rate { display: flex; align-items: center; gap: 6px; }
  .lib-num { display: inline-flex; align-items: center; gap: 3px; font-size: 12px; color: var(--muted-2); font-variant-numeric: tabular-nums; }
  .lib-num :global(svg) { fill: var(--muted); stroke: var(--muted); }
  .cover-sub { font-size: 12px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  input[type='checkbox'] { width: 16px; height: 16px; accent-color: var(--accent); }
  /* after .hcell / .title-cell (position, alignment) */
  .hcell.sticky-num, .hcell.sticky-title { position: sticky; }
  .title-cell.sticky-title { align-items: center; }
</style>
