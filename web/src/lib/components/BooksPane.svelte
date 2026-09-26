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
  import { t, tn, i18nState } from '../i18n';
  import { getPref, setPref } from '../stores/prefs.svelte';
  import { myRatings } from '../stores/myRatings.svelte';
  import { COL_LIMITS, colWidth, setColWidth, type ColKey } from '../stores/layout.svelte';
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
    booksCount: number;
    seriesCount?: number;
    anthologies?: number;
    /** top co-authors of an author (see AuthorSummary) */
    coauthors?: Coauthor[];
    coauthorCount?: number;
    /** a Follow button for this author / series */
    follow?: { kind: 'author' | 'series'; id: number };
  };

  let {
    lib, scope, selectedBookId, onPick, onOpenSend, onOpenShelf, header, onBack, onCounts,
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
  // entries are column keys, or `!size` / `!added` for a default column the user hid
  const extraColumns = $derived(
    new Set<string>(getPref<string[]>(colsPrefKey, scope.kind === 'author' ? [] : ['author'])),
  );
  const hasRatingCol = $derived(RATING_COLS.some((k) => extraColumns.has(k)));
  /** Size and Added are on by default, except when rating columns are on (the table then
   *  fits beside the details pane); an explicit choice in the Columns menu wins. */
  function colShown(c: OptCol): boolean {
    if (c === 'size' || c === 'added') {
      if (extraColumns.has(c)) return true;
      if (extraColumns.has(`!${c}`)) return false;
      return !hasRatingCol;
    }
    return extraColumns.has(c);
  }
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
  let coauthorsOpen = $state(false);
  let collapsed = $state<Set<string>>(new Set());
  let collapseInitFor = '';
  let lastClickedIndex = -1;
  let listRef = $state<{ reveal: (i: number) => void; resetX: () => void } | undefined>();
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
    collapsed = n > COLLAPSE_ABOVE ? new Set(groups.map((g) => g.key)) : new Set();
  });
  const effectiveCollapsed = $derived(q ? new Set<string>() : collapsed);
  const allCollapsed = $derived(showGroupHeads && groups.every((g) => effectiveCollapsed.has(g.key)));

  function toggleCollapse(key: string) {
    const s = new Set(collapsed);
    if (s.has(key)) s.delete(key); else s.add(key);
    collapsed = s;
  }
  function setAllCollapsed(on: boolean) {
    collapsed = on ? new Set(groups.map((g) => g.key)) : new Set();
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
      if (collapsed.has(g.key)) { const s = new Set(collapsed); s.delete(g.key); collapsed = s; }
      tick().then(() => {
        const i = flatRows.findIndex((fr) => fr.kind === 'row' && fr.row.book.id === id);
        if (i >= 0) listRef?.reveal(Math.max(0, i));
      });
    });
  });

  type FlatRow = { kind: 'group'; group: Group } | { kind: 'row'; row: Row; groupKey: string }
    | { kind: 'edition'; ed: Edition; of: number };
  const flatRows = $derived.by<FlatRow[]>(() => {
    const out: FlatRow[] = [];
    for (const g of groups) {
      if (showGroupHeads) out.push({ kind: 'group', group: g });
      if (showGroupHeads && effectiveCollapsed.has(g.key)) continue;
      for (const r of g.rows) {
        out.push({ kind: 'row', row: r, groupKey: g.key });
        if (openEditions.has(r.book.id)) {
          for (const ed of editionsOf.get(r.book.id) ?? []) if (ed.id !== r.book.id) out.push({ kind: 'edition', ed, of: r.book.id });
        }
      }
    }
    return out;
  });
  function editionLine(e: Edition): string {
    return [e.ext.toUpperCase(), formatSize(e.size), formatDate(e.date, i18nState.lang), e.lang, e.libRating ? `★${e.libRating}` : '', e.note ?? '']
      .filter(Boolean).join(' · ');
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
  }

  // Keyboard: ↑/↓ move the current book, Space ticks it, ←/→ fold/unfold its series group.
  function tableKeydown(e: KeyboardEvent) {
    if ((e.target as HTMLElement).closest('input, button:not(.title-btn), [role=separator]')) return;
    const rows = flatRows;
    let cur = rows.findIndex((fr) => fr.kind === 'row' && fr.row.book.id === selectedBookId);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const d = e.key === 'ArrowDown' ? 1 : -1;
      let i = cur < 0 ? (d > 0 ? -1 : rows.length) : cur;
      do { i += d; } while (i >= 0 && i < rows.length && rows[i].kind !== 'row');
      if (i >= 0 && i < rows.length) {
        const fr = rows[i];
        if (fr.kind === 'row') { onPick(fr.row.book.id); lastClickedIndex = i; }
        listRef?.reveal(i);
      }
    } else if (e.key === ' ' && cur >= 0) {
      e.preventDefault();
      const fr = rows[cur];
      if (fr.kind === 'row') toggle(lib, fr.row.book.id);
    } else if ((e.key === 'ArrowLeft' || e.key === 'ArrowRight') && showGroupHeads && cur >= 0) {
      const fr = rows[cur];
      if (fr.kind !== 'row') return;
      e.preventDefault();
      if (e.key === 'ArrowLeft') { const s = new Set(collapsed); s.add(fr.groupKey); collapsed = s; }
    }
  }

  // ---- columns ---------------------------------------------------------------------
  let liveCols = $state<Partial<Record<ColKey, number>>>({});
  const showNum = $derived(fullScope);
  type Col = ColKey | 'title';
  const columns = $derived.by<Col[]>(() => {
    const c: Col[] = [];
    if (showNum) c.push('num');
    c.push('title');
    for (const k of ['author', 'series', 'genre', 'language', 'format', 'size', 'added', 'rating', 'libRating', 'extRating'] as const) if (colShown(k)) c.push(k);
    return c;
  });
  const titleIndex = $derived(columns.indexOf('title'));
  /** below this width the table scrolls sideways instead of squeezing the title */
  const w = (k: ColKey) => liveCols[k] ?? colWidth(k);
  const TITLE_MIN = 220;
  const tableMinWidth = $derived(
    32 + TITLE_MIN + columns.filter((c) => c !== 'title').reduce((n, c) => n + w(c as ColKey), 0) + 8 * columns.length + 28,
  );
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
    const on = colShown(c);
    s.delete(c); s.delete(`!${c}`);
    if (c === 'size' || c === 'added') s.add(on ? `!${c}` : c);
    else if (!on) s.add(c);
    setPref(colsPrefKey, [...s]);
  }

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
  <div class="col-menu" class:phone-menu={phone} role="menu">
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

{#snippet colHead(c: Col, i: number)}
  <span class="hcell" class:sticky-num={c === 'num'} class:sticky-title={c === 'title'} class:right={c === 'size' || c === 'added' || c === 'rating' || c === 'libRating' || c === 'extRating'}>
    <span class="hlabel" title={c === 'libRating' ? t('books.col.libRating') : undefined}>{colLabel[c]}</span>
    {#if c !== 'title'}
      {@const k = c as ColKey}
      {#if i < titleIndex}
        <Splitter
          class="col-split at-right"
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
        <span>{tn('browse.booksCount', header.booksCount)}</span>
        {#if header.seriesCount}<span>· {tn('browse.seriesCount', header.seriesCount)}</span>{/if}
        {#if header.anthologies}<span>· {tn('browse.inAnthologies', header.anthologies)}</span>{/if}
      </div>
      {#if shownCoauthors.length || moreCoauthors}
        <div class="coauthors">
          <span class="lbl">{t('authors.alsoWith')}</span>
          <span class="names">
            {#each shownCoauthors as a, i (a.id)}{#if i > 0}{', '}{/if}<a href="/l/{lib}/authors/{a.id}" data-link title={tn('authors.sharedBooks', a.books)}>{a.name}</a>{/each}
          </span>
          {#if moreCoauthors}
            <div class="more-wrap">
              <button type="button" class="more-btn" aria-haspopup="dialog" aria-expanded={coauthorsOpen} onclick={() => (coauthorsOpen = !coauthorsOpen)}>
                {shownCoauthors.length ? t('authors.andMore', { count: moreCoauthors }) : tn('authors.coauthorsCount', moreCoauthors)}
              </button>
              {#if coauthorsOpen && scope.kind === 'author'}
                <CoauthorsPopover {lib} authorId={scope.id} onClose={() => (coauthorsOpen = false)} />
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
      <button type="button" class="tbtn" onclick={() => setAllCollapsed(!allCollapsed)}>
        {allCollapsed ? t('books.expandAll') : t('books.collapseAll')}
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
      <button type="button" class="tbtn" class:on={activeFilterCount > 0} aria-haspopup="true" aria-expanded={filterMenuOpen} onclick={() => (filterMenuOpen = !filterMenuOpen)}>
        <Icon name="filter" size={14} /><span class="lbl-f">{t('books.filter')}</span>{#if activeFilterCount}<span class="badge">{activeFilterCount}</span>{/if}
      </button>
      {#if filterMenuOpen}{@render filterMenu(isMobile)}{/if}
    </div>
    {#if countLabel}<span class="shown">{countLabel}</span>{/if}
    <div class="spacer"></div>
    {#if !isMobile}
      <div class="col-chooser">
        <button type="button" class="tbtn" onclick={() => (columnMenuOpen = !columnMenuOpen)} aria-haspopup="true" aria-expanded={columnMenuOpen}>
          {t('books.columns')}<Icon name="chevronDown" size={14} />
        </button>
        {#if columnMenuOpen}
          <div class="col-menu" role="menu">
            {#each OPT_COLUMNS as c (c)}
              <label>
                <input type="checkbox" checked={colShown(c)} onchange={() => toggleColumn(c)} />
                {t(`books.col.${c}`)}
              </label>
            {/each}
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
    <div class="mobile-list">
      <VirtualList items={flatRows} itemHeight={64} overscan={6} onRangeChange={onRowRangeChange}>
        {#snippet row(fr)}
          {#if fr.kind === 'group'}
            <button type="button" class="m-group" aria-expanded={!effectiveCollapsed.has(fr.group.key)} onclick={() => toggleCollapse(fr.group.key)}>
              <span class="chev" class:open={!effectiveCollapsed.has(fr.group.key)}><Icon name="chevronRight" size={14} /></span>
              <span class="gname">{fr.group.name}</span><span class="gcount">{fr.group.count}</span>
            </button>
          {:else if fr.kind === 'edition'}
            {@const e = fr.ed}
            <div class="m-row m-edition" role="row" tabindex="0" data-testid="edition-row" class:checked={isSelected(lib, e.id)}
              onclick={() => onPick(e.id)} onkeydown={(ev) => { if (ev.key === 'Enter') onPick(e.id); }}>
              <span class="ed-mark"><Icon name="layers" size={14} /></span>
              <div class="m-info"><span class="m-meta">{editionLine(e)}</span></div>
              <input type="checkbox" aria-label={t('books.select', { title: e.title })} checked={isSelected(lib, e.id)}
                onclick={(ev) => ev.stopPropagation()} onchange={() => toggle(lib, e.id)} />
            </div>
          {:else}
            {@const r = fr.row}
            <div
              class="m-row"
              role="row"
              tabindex="0"
              class:checked={isSelected(lib, r.book.id)}
              class:deleted={r.book.deleted}
              onclick={() => onPick(r.book.id)}
              onkeydown={(e) => { if (e.key === 'Enter') onPick(r.book.id); }}
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
    <div class="table-wrap" class:scrolled-x={scrolledX} style="--sticky-num: {stickyNum}px; --sticky-title: {stickyTitle}px">
      <div class="head-wrap" bind:this={headWrap}>
        <div class="brow head" role="row" style="grid-template-columns: {gridColumns}; min-width: {tableMinWidth}px">
          <span class="cell-check"><input type="checkbox" aria-label={t('books.selectAll')} checked={allChecked} onchange={toggleAll} /></span>
          {#each columns as c, i (c)}{@render colHead(c, i)}{/each}
        </div>
      </div>
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div class="scroll" tabindex="0" role="grid" aria-label={t('books.list')} onkeydown={tableKeydown}>
        <VirtualList bind:this={listRef} items={flatRows} itemHeight={ROW_H} onRangeChange={onRowRangeChange} scrollX {onScrollX}>
          {#snippet row(fr, fi)}
            {#if fr.kind === 'group'}
              {@const open = !effectiveCollapsed.has(fr.group.key)}
              <div class="group-head" role="row">
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
              <div class="brow edition-row" role="row" tabindex="-1" data-testid="edition-row"
                class:selected={e.id === selectedBookId} class:checked={isSelected(lib, e.id)} class:deleted={e.deleted}
                style="min-width: {tableMinWidth}px" onclick={() => onPick(e.id)}>
                <span class="cell-check"><input type="checkbox" aria-label={t('books.select', { title: e.title })} checked={isSelected(lib, e.id)}
                  onclick={(ev) => ev.stopPropagation()} onchange={() => toggle(lib, e.id)} /></span>
                <span class="ed-line"><Icon name="layers" size={13} /><span class="ellipsis">{editionLine(e)}</span></span>
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
      {#each visibleBooks as b (b.id)}
        <button type="button" class="cover-card" class:selected={b.id === selectedBookId} onclick={() => onPick(b.id)} title={b.title}>
          <div class="cover-wrap">
            <CoverThumb {lib} bookId={b.id} title={b.title} width={140} height={200} />
            {#if isSelected(lib, b.id)}
              <span class="check-badge"><Icon name="check" size={14} /></span>
            {/if}
            <input
              type="checkbox"
              class="grid-check"
              aria-label={t('books.select', { title: b.title })}
              checked={isSelected(lib, b.id)}
              onclick={(e) => e.stopPropagation()}
              onchange={() => toggle(lib, b.id)}
            />
          </div>
          <span class="cover-title">{b.title}</span>
          {#if b.kidsAge !== null && b.kidsAge !== undefined || b.extRating}
            <span class="cover-rate"><KidsBadge age={b.kidsAge} />{#if b.extRating}<ExtRating value={b.extRating} />{/if}</span>
          {/if}
          {#if scope.kind !== 'author'}<span class="cover-sub">{authorsShort(b)}</span>{/if}
        </button>
      {/each}
      {#if nextCursor}<div class="grid-sentinel" use:sentinel>{fetchingMore ? t('common.loading') : ''}</div>{/if}
    </div>
  {/if}

  <SelectionBar
    count={selectedCount(lib)}
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
  .scope-header h1 {
    margin: 2px 0 2px; font-family: var(--font-display); font-size: 24px; font-weight: 600; line-height: 1.25;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .h1-row { display: flex; align-items: center; gap: 12px; min-width: 0; }
  .h1-row h1 { min-width: 0; }
  .ed-tag { border: none; cursor: pointer; font: inherit; font-size: 11px; color: var(--accent-soft-ink); background: var(--accent-soft); border-radius: 4px; padding: 1px 6px; white-space: nowrap; }
  .ed-tag:hover { text-decoration: underline; }
  .ed-tag[aria-expanded='true'] { background: var(--accent); color: #fff; }
  .edition-row { display: flex; --row-bg: var(--surface-alt); }
  .ed-line { display: flex; align-items: center; gap: 8px; padding-left: 48px; font-size: 13px; color: var(--muted-2); min-width: 0; }
  .m-edition { background: var(--surface-alt); }
  .ed-mark { width: 36px; display: flex; justify-content: center; color: var(--muted); flex-shrink: 0; }
  .counts { display: flex; gap: 4px; font-size: 13px; color: var(--muted); white-space: nowrap; overflow: hidden; }
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
  .scroll :global(.vlist) { flex-grow: 1; min-height: 0; }
  .brow { --row-bg: var(--surface); background: var(--row-bg); display: grid; align-items: center; height: 40px; padding: 0 12px 0 16px; border-bottom: 1px solid var(--line-soft); font-size: 14px; cursor: default; gap: 8px; }
  .head-wrap { overflow: hidden; flex-shrink: 0; }
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
  .grid-sentinel { grid-column: 1 / -1; height: 24px; text-align: center; font-size: 12px; color: var(--muted); }
  .cover-card { all: unset; display: flex; flex-direction: column; gap: 4px; cursor: pointer; min-width: 0; }
  .cover-card:focus-visible { outline: 2px solid var(--focus); outline-offset: 4px; }
  .cover-wrap { position: relative; margin-bottom: 4px; }
  .cover-wrap :global(.cover) { width: 100% !important; aspect-ratio: 2/3; height: auto !important; }
  .cover-card.selected .cover-wrap :global(.cover) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .check-badge { position: absolute; top: 8px; right: 8px; width: 22px; height: 22px; border-radius: 11px; background: #FFF; display: flex; align-items: center; justify-content: center; color: var(--accent); }
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
