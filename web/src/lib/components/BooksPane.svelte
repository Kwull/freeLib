<script lang="ts">
  import type { Book } from '../api/types';
  import { api } from '../api/client';
  import Icon from './Icon.svelte';
  import CoverThumb from './CoverThumb.svelte';
  import Rating from './Rating.svelte';
  import SelectionBar from './SelectionBar.svelte';
  import { formatSize, formatDate } from '../utils/format';
  import { t, i18nState } from '../i18n';
  import {
    isSelected, selectedCount, toggle, toggleMany, clear as clearSelection, selectedIds,
  } from '../stores/selection.svelte';

  type Scope =
    | { kind: 'author'; id: number; groupable: true }
    | { kind: 'series'; id: number; groupable: false }
    | { kind: 'genre'; id: number; groupable: false }
    | { kind: 'shelf'; id: number; groupable: false }
    | { kind: 'since'; date: string; groupable: false };

  let {
    lib, scope, selectedBookId, onPick, onOpenSend, onOpenShelf,
  }: {
    lib: number;
    scope: Scope;
    selectedBookId: number | null;
    onPick: (id: number) => void;
    onOpenSend: (ids: number[]) => void;
    onOpenShelf: (ids: number[]) => void;
  } = $props();

  const ALL_COLUMNS = ['author', 'series', 'genre', 'language', 'format'] as const;
  type OptCol = (typeof ALL_COLUMNS)[number];

  let books = $state<Book[]>([]);
  let loading = $state(true);
  let genreNames = $state<Map<number, string>>(new Map());
  $effect(() => { api.genres(lib).then((gs) => (genreNames = new Map(gs.map((g) => [g.id, g.name])))); });
  let view = $state<'table' | 'grid'>('table');
  let grouping = $state(true);
  let hideDeleted = $state(true);
  let columnMenuOpen = $state(false);
  let extraColumns = $state<Set<OptCol>>(new Set(scope.kind === 'author' ? [] : ['author']));
  let lastClickedIndex = -1;

  $effect(() => {
    let cancelled = false;
    loading = true;
    books = [];
    async function load() {
      const params: Record<string, unknown> = { limit: 2000, deleted: !hideDeleted };
      if (scope.kind === 'author') params.author = scope.id;
      else if (scope.kind === 'series') params.series = scope.id;
      else if (scope.kind === 'genre') params.genre = scope.id;
      else if (scope.kind === 'shelf') params.shelf = scope.id;
      else params.since = scope.date;

      let cursor: string | undefined;
      const acc: Book[] = [];
      for (let page = 0; page < 25; page++) {
        const res = await api.books(lib, { ...params, cursor } as any);
        if (cancelled) return;
        acc.push(...res.books);
        books = acc.slice();
        if (!res.nextCursor) break;
        cursor = res.nextCursor;
      }
      loading = false;
    }
    load();
    return () => { cancelled = true; };
  });

  type Row = { book: Book; num: number | string };
  type Group = { key: string; name: string; count: number; rows: Row[] };

  const groups = $derived.by<Group[]>(() => {
    if (!(scope.kind === 'author' && grouping)) {
      return [{ key: '_all', name: '', count: books.length, rows: books.map((b, i) => ({ book: b, num: i + 1 })) }];
    }
    const bySeries = new Map<string, Book[]>();
    const order: string[] = [];
    for (const b of books) {
      const key = b.series ? `s${b.series.id}` : '_none';
      if (!bySeries.has(key)) { bySeries.set(key, []); order.push(key); }
      bySeries.get(key)!.push(b);
    }
    return order.map((key) => {
      const list = bySeries.get(key)!;
      const name = key === '_none' ? t('books.outsideSeries') : list[0].series!.name;
      return {
        key, name, count: list.length,
        rows: list.map((b) => ({ book: b, num: b.serno ?? '' })),
      };
    });
  });

  const allIds = $derived(books.map((b) => b.id));
  const allChecked = $derived(allIds.length > 0 && allIds.every((id) => isSelected(lib, id)));
  const anyChecked = $derived(selectedCount(lib) > 0);

  function toggleAll() {
    toggleMany(lib, allIds, !allChecked);
  }
  function toggleGroup(ids: number[]) {
    const on = !ids.every((id) => isSelected(lib, id));
    toggleMany(lib, ids, on);
  }
  function rowClick(e: MouseEvent, book: Book, flatIndex: number) {
    if (e.shiftKey && lastClickedIndex >= 0) {
      const flat = groups.flatMap((g) => g.rows.map((r) => r.book));
      const [a, b] = [lastClickedIndex, flatIndex].sort((x, y) => x - y);
      toggleMany(lib, flat.slice(a, b + 1).map((x) => x.id), true);
    } else {
      onPick(book.id);
    }
    lastClickedIndex = flatIndex;
  }

  function flatIndexOf(bookId: number): number {
    let i = 0;
    for (const g of groups) for (const r of g.rows) { if (r.book.id === bookId) return i; i++; }
    return -1;
  }

  const gridColumns = $derived.by(() => {
    // checkbox, num, title(flex), then optional columns, rating
    const cols = ['36px', '36px', 'minmax(0,1fr)'];
    if (extraColumns.has('author')) cols.push('140px');
    if (extraColumns.has('series')) cols.push('140px');
    if (extraColumns.has('genre')) cols.push('120px');
    if (extraColumns.has('language')) cols.push('70px');
    if (extraColumns.has('format')) cols.push('70px');
    cols.push('64px', '92px', '84px');
    return cols.join(' ');
  });

  function toggleColumn(c: OptCol) {
    const s = new Set(extraColumns);
    if (s.has(c)) s.delete(c); else s.add(c);
    extraColumns = s;
  }

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
    const parts = [r.num ? `#${r.num}` : '', r.book.ext.toUpperCase(), formatSize(r.book.size)];
    return parts.filter(Boolean).join(' · ');
  }
</script>

<main class="books-pane">
  <div class="toolbar">
    <div class="row">
      <button type="button" class="tbtn" onclick={() => (hideDeleted = !hideDeleted)}>
        {hideDeleted ? t('books.hideDeleted') : t('books.showDeleted')}
      </button>
      <div class="spacer"></div>
      {#if scope.kind === 'author'}
        <button type="button" class="tbtn group-btn" onclick={() => (grouping = !grouping)}>
          {grouping ? t('books.groupSeries') : t('books.groupNone')}
          <Icon name="chevronDown" size={14} />
        </button>
      {/if}
      <div class="col-chooser">
        <button type="button" class="tbtn" onclick={() => (columnMenuOpen = !columnMenuOpen)} aria-haspopup="true" aria-expanded={columnMenuOpen}>
          {t('books.columns')}<Icon name="chevronDown" size={14} />
        </button>
        {#if columnMenuOpen}
          <div class="col-menu" role="menu">
            {#each ALL_COLUMNS as c (c)}
              <label>
                <input type="checkbox" checked={extraColumns.has(c)} onchange={() => toggleColumn(c)} />
                {t(`books.col.${c}`)}
              </label>
            {/each}
          </div>
        {/if}
      </div>
      <div role="group" aria-label="View" class="view-toggle">
        <button type="button" aria-label={t('books.viewTable')} aria-pressed={view === 'table'} class:active={view === 'table'} onclick={() => (view = 'table')}>
          <Icon name="table" size={16} />
        </button>
        <button type="button" aria-label={t('books.viewGrid')} aria-pressed={view === 'grid'} class:active={view === 'grid'} onclick={() => (view = 'grid')}>
          <Icon name="grid" size={16} />
        </button>
      </div>
    </div>
  </div>

  {#if loading && books.length === 0}
    <div class="empty">{t('common.loading')}</div>
  {:else if books.length === 0}
    <div class="empty">{t('search.noResults')}</div>
  {:else if isMobile}
    <div class="mobile-list">
      {#each groups as g (g.key)}
        {#if g.name}
          <div class="m-group">{g.name}<span class="gcount">{g.count}</span></div>
        {/if}
        {#each g.rows as r (r.book.id)}
          <div
            class="m-row"
            role="row"
            tabindex="0"
            class:checked={isSelected(lib, r.book.id)}
            onclick={() => onPick(r.book.id)}
            onkeydown={(e) => { if (e.key === 'Enter') onPick(r.book.id); }}
          >
            <CoverThumb {lib} bookId={r.book.id} title={r.book.title} width={36} height={52} />
            <div class="m-info">
              <span class="m-title">{r.book.title}</span>
              <span class="m-meta">{rowMeta(r)}</span>
            </div>
            <input
              type="checkbox"
              aria-label={`Select ${r.book.title}`}
              checked={isSelected(lib, r.book.id)}
              onclick={(e) => e.stopPropagation()}
              onchange={() => toggle(lib, r.book.id)}
            />
          </div>
        {/each}
      {/each}
    </div>
  {:else if view === 'table'}
    <div class="table-wrap">
      <div class="brow head" style="grid-template-columns: {gridColumns}">
        <input type="checkbox" aria-label={t('books.selectAll')} checked={allChecked} onchange={toggleAll} />
        <span>{t('books.col.no')}</span>
        <span>{t('books.col.title')}</span>
        {#if extraColumns.has('author')}<span>{t('books.col.author')}</span>{/if}
        {#if extraColumns.has('series')}<span>{t('books.col.series')}</span>{/if}
        {#if extraColumns.has('genre')}<span>{t('books.col.genre')}</span>{/if}
        {#if extraColumns.has('language')}<span>{t('books.col.language')}</span>{/if}
        {#if extraColumns.has('format')}<span>{t('books.col.format')}</span>{/if}
        <span style="text-align:right">{t('books.col.size')}</span>
        <span style="text-align:right">{t('books.col.added')}</span>
        <span style="text-align:right">{t('books.col.rating')}</span>
      </div>
      <div class="scroll">
        {#each groups as g (g.key)}
          {#if g.name}
            <div class="group-head">
              <input
                type="checkbox"
                aria-label={`Select series ${g.name}`}
                checked={g.rows.every((r) => isSelected(lib, r.book.id))}
                onchange={() => toggleGroup(g.rows.map((r) => r.book.id))}
              />
              <span class="gname">{g.name}</span>
              <span class="gcount">{g.count}</span>
            </div>
          {/if}
          {#each g.rows as r (r.book.id)}
            <div
              class="brow"
              role="row"
              tabindex="0"
              class:selected={r.book.id === selectedBookId}
              class:checked={isSelected(lib, r.book.id)}
              style="grid-template-columns: {gridColumns}"
              onclick={(e) => rowClick(e, r.book, flatIndexOf(r.book.id))}
              onkeydown={(e) => { if (e.key === 'Enter') { onPick(r.book.id); } }}
            >
              <input
                type="checkbox"
                aria-label={`Select ${r.book.title}`}
                checked={isSelected(lib, r.book.id)}
                onclick={(e) => e.stopPropagation()}
                onchange={() => toggle(lib, r.book.id)}
              />
              <span class="muted">{r.num}</span>
              <button type="button" class="title-btn" class:strong={r.book.id === selectedBookId}>{r.book.title}</button>
              {#if extraColumns.has('author')}<span class="muted ellipsis">{r.book.authors.map((a) => a.name).join(', ')}</span>{/if}
              {#if extraColumns.has('series')}<span class="muted ellipsis">{r.book.series?.name ?? ''}</span>{/if}
              {#if extraColumns.has('genre')}<span class="muted ellipsis">{genreNames.get(r.book.genres[0]) ?? ''}</span>{/if}
              {#if extraColumns.has('language')}<span class="muted">{r.book.lang}</span>{/if}
              {#if extraColumns.has('format')}<span class="muted">{r.book.ext}</span>{/if}
              <span class="muted" style="text-align:right">{formatSize(r.book.size)}</span>
              <span class="muted" style="text-align:right">{formatDate(r.book.date, i18nState.lang)}</span>
              <span style="display:flex;justify-content:flex-end"><Rating value={r.book.rating} /></span>
            </div>
          {/each}
        {/each}
      </div>
    </div>
  {:else}
    <div class="grid-view">
      {#each books as b (b.id)}
        <button type="button" class="cover-card" class:selected={b.id === selectedBookId} onclick={() => onPick(b.id)}>
          <div class="cover-wrap">
            <CoverThumb {lib} bookId={b.id} title={b.title} width={140} height={200} />
            {#if isSelected(lib, b.id)}
              <span class="check-badge"><Icon name="check" size={14} /></span>
            {/if}
            <input
              type="checkbox"
              class="grid-check"
              aria-label={`Select ${b.title}`}
              checked={isSelected(lib, b.id)}
              onclick={(e) => e.stopPropagation()}
              onchange={() => toggle(lib, b.id)}
            />
          </div>
          <span class="cover-title">{b.title}</span>
        </button>
      {/each}
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
  .books-pane { flex-grow: 1; min-width: 0; display: flex; flex-direction: column; background: var(--surface); position: relative; min-height: 0; }
  .toolbar { padding: 12px 24px; border-bottom: 1px solid var(--line); flex-shrink: 0; }
  .row { display: flex; align-items: center; gap: 8px; }
  .spacer { flex-grow: 1; }
  .tbtn {
    display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 10px;
    border: 1px solid var(--border); border-radius: 6px; background: var(--surface); color: var(--muted-2); font-size: 13px;
  }
  .col-chooser { position: relative; }
  .col-menu {
    position: absolute; top: 38px; right: 0; z-index: 10; background: var(--surface); border: 1px solid var(--line);
    border-radius: 8px; box-shadow: 0 8px 24px rgba(0,0,0,.15); padding: 8px; display: flex; flex-direction: column; gap: 4px; min-width: 160px;
  }
  .col-menu label { display: flex; align-items: center; gap: 8px; font-size: 13px; padding: 4px 6px; border-radius: 4px; }
  .col-menu label:hover { background: var(--surface-hover); }
  .view-toggle { display: flex; border: 1px solid var(--border); border-radius: 6px; overflow: hidden; }
  .view-toggle button { width: 36px; height: 30px; border: none; background: var(--surface); color: var(--muted-2); display: flex; align-items: center; justify-content: center; }
  .view-toggle button + button { border-left: 1px solid var(--border); }
  .view-toggle button.active { background: var(--surface-hover); }
  .empty { flex-grow: 1; display: flex; align-items: center; justify-content: center; color: var(--muted); font-size: 14px; }
  .mobile-list { flex-grow: 1; overflow-y: auto; padding-bottom: 64px; }
  .m-group { padding: 14px 16px 6px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; display: flex; gap: 8px; background: var(--surface-alt); }
  .m-group .gcount { font-weight: 400; }
  .m-row { display: flex; align-items: center; gap: 12px; padding: 8px 16px; min-height: 64px; box-sizing: border-box; }
  .m-row.checked { background: var(--accent-soft); }
  .m-info { flex-grow: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .m-title { font-size: 15px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .m-meta { font-size: 12px; color: var(--muted); }
  .m-row input[type='checkbox'] { width: 22px; height: 22px; flex-shrink: 0; }
  @media (max-width: 900px) {
    .toolbar .col-chooser, .toolbar .view-toggle, .toolbar .group-btn { display: none; }
  }
  .table-wrap { flex-grow: 1; overflow: hidden; display: flex; flex-direction: column; min-height: 0; }
  /* Leaves room so the floating selection bar never covers the last rows. */
  .scroll { flex-grow: 1; overflow-y: auto; padding-bottom: 64px; scroll-padding-bottom: 64px; }
  .brow { display: grid; align-items: center; height: 40px; padding: 0 12px; border-bottom: 1px solid var(--line-soft); font-size: 14px; cursor: default; gap: 8px; }
  .brow.head { height: 34px; font-size: 12px; font-weight: 600; color: var(--muted); background: var(--surface-alt); }
  .brow:not(.head):hover { background: var(--row-hover); }
  .brow.checked { background: #F3F7F6; }
  :global(:root[data-theme='dark']) .brow.checked { background: #1E3230; }
  .brow.selected { background: var(--accent-soft); }
  .group-head { display: flex; align-items: center; gap: 10px; height: 36px; padding: 0 12px; background: var(--page); border-bottom: 1px solid var(--line-soft); font-size: 13px; }
  .gname { font-weight: 600; }
  .gcount { color: var(--muted); }
  .muted { color: var(--muted); }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .title-btn { all: unset; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; cursor: pointer; }
  .title-btn.strong { font-weight: 600; }
  .grid-view { flex-grow: 1; overflow-y: auto; padding: 20px 24px; display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 20px 16px; align-content: start; }
  .cover-card { all: unset; display: flex; flex-direction: column; gap: 8px; cursor: pointer; }
  .cover-wrap { position: relative; }
  .cover-wrap :global(.cover) { width: 100% !important; aspect-ratio: 2/3; height: auto !important; }
  .cover-card.selected .cover-wrap :global(.cover) { outline: 2px solid var(--accent); outline-offset: 2px; }
  .check-badge { position: absolute; top: 8px; right: 8px; width: 22px; height: 22px; border-radius: 11px; background: #FFF; display: flex; align-items: center; justify-content: center; color: var(--accent); }
  .grid-check { position: absolute; top: 8px; left: 8px; width: 16px; height: 16px; }
  .cover-title { font-size: 13px; line-height: 1.3; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  input[type='checkbox'] { width: 16px; height: 16px; accent-color: var(--accent); }
</style>
