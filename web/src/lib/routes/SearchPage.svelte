<script lang="ts">
  import { api, errorText } from '../api/client';
  import type { Book, SearchResponse } from '../api/types';
  import { navigate } from '../router.svelte';
  import { t, tn, i18nState } from '../i18n';
  import Icon from '../components/Icon.svelte';
  import CoverThumb from '../components/CoverThumb.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import ShelfDialog from '../components/ShelfDialog.svelte';
  import DetailsPane from '../components/DetailsPane.svelte';
  import { normalize } from '../utils/normalize';
  import { formatDate } from '../utils/format';
  import RatingFilters from '../components/RatingFilters.svelte';
  import ExtRating from '../components/ExtRating.svelte';
  import KidsBadge from '../components/KidsBadge.svelte';
  import { getPref, setPref } from '../stores/prefs.svelte';
  import {
    emptyRatingFilters, ratingFilterCount, ratingParams, type RatingFilters as RatingFiltersT, type RatingSortKey,
  } from '../utils/ratings';

  let { lib, q }: { lib: number; q: string } = $props();

  let query = $state('');
  let result = $state<SearchResponse | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let genreFilter = $state<Set<number>>(new Set());
  let langFilter = $state<Set<string>>(new Set());
  let extFilter = $state<Set<string>>(new Set());
  let send = $state<{ ids: number[]; device?: number } | null>(null);
  let shelfIds = $state<number[] | null>(null);
  let genresById = $state<Map<number, string>>(new Map());
  let allGenres = $state(false);
  let facetsOpen = $state(false);
  let selectedBookId = $state<number | null>(null);
  let expanded = $state<Set<string>>(new Set());
  let ratingFilters = $state<RatingFiltersT>(emptyRatingFilters());
  type SearchSort = 'relevance' | RatingSortKey;
  const sortKey = $derived(getPref<SearchSort>('sort.search', 'relevance'));

  $effect(() => { query = q; selectedBookId = null; expanded = new Set(); });
  $effect(() => {
    const lang = i18nState.lang;
    api.genres(lib, lang).then((gs) => (genresById = new Map(gs.map((g) => [g.id, g.name])))).catch(() => {});
  });

  $effect(() => {
    if (!q || q.trim().length < 2) { result = null; loading = false; return; }
    let cancelled = false;
    loading = true;
    error = null;
    api.search(lib, {
      q,
      kind: 'all',
      genre: [...genreFilter].join(',') || undefined,
      lang: [...langFilter].join(',') || undefined,
      ext: [...extFilter].join(',') || undefined,
      ...ratingParams(ratingFilters, sortKey === 'relevance' ? null : sortKey),
    }).then((r) => { if (!cancelled) result = r; })
      .catch((e) => { if (!cancelled) error = errorText(e); })
      .finally(() => { if (!cancelled) loading = false; });
    return () => { cancelled = true; };
  });

  function submit() {
    navigate(`/l/${lib}/search?q=${encodeURIComponent(query.trim())}`);
  }
  function toggleSet<T>(set: Set<T>, v: T): Set<T> {
    const next = new Set(set);
    if (next.has(v)) next.delete(v); else next.add(v);
    return next;
  }
  function clearFilters() {
    genreFilter = new Set(); langFilter = new Set(); extFilter = new Set();
    ratingFilters = emptyRatingFilters();
  }
  const filterCount = $derived(genreFilter.size + langFilter.size + extFilter.size + ratingFilterCount(ratingFilters));

  function highlight(title: string): { pre: string; hit: string; post: string } | null {
    const words = normalize(q).split(' ').filter(Boolean).sort((a, b) => b.length - a.length);
    const norm = normalize(title);
    // only when normalization kept the length (index positions then match the title)
    if (norm.length !== title.length) return null;
    for (const w of words) {
      const idx = norm.indexOf(w);
      if (idx >= 0) return { pre: title.slice(0, idx), hit: title.slice(idx, idx + w.length), post: title.slice(idx + w.length) };
    }
    return null;
  }

  // Flibusta has many editions of the same book (same title and authors): show one row per
  // work, with the other editions behind a toggle.
  type Work = { key: string; first: Book; others: Book[] };
  const works = $derived.by<Work[]>(() => {
    if (!result) return [];
    const map = new Map<string, Work>();
    const out: Work[] = [];
    for (const b of result.books) {
      const key = `${normalize(b.title)}|${b.authors.map((a) => a.id).join(',')}`;
      const w = map.get(key);
      if (w) w.others.push(b);
      else { const nw = { key, first: b, others: [] }; map.set(key, nw); out.push(nw); }
    }
    return out;
  });

  function pick(b: Book) {
    if (window.innerWidth < 900) navigate(`/l/${lib}/book/${b.id}`);
    else selectedBookId = b.id;
  }
  const authorsLine = (b: Book) =>
    b.authors.length > 3 ? `${b.authors.slice(0, 2).map((a) => a.name).join(', ')} ${t('books.andMore', { count: b.authors.length - 2 })}` : b.authors.map((a) => a.name).join(', ');
  const genreLine = (b: Book) => b.genres.map((g) => genresById.get(g)).filter(Boolean)[0] ?? '';
  const facetGenres = $derived(result ? (allGenres ? result.facets.genre : result.facets.genre.slice(0, 8)) : []);
</script>

{#snippet bookRow(b: Book, w: Work | null, edition: boolean)}
  <div class="res-row" class:edition class:selected={b.id === selectedBookId}>
    {#if !edition}<CoverThumb {lib} bookId={b.id} title={b.title} />{:else}<span></span>{/if}
    <div class="info">
      <a href="/l/{lib}/book/{b.id}" class="title" onclick={(e) => { e.preventDefault(); pick(b); }}>
        {#if !edition && highlight(b.title)}
          {@const h = highlight(b.title)}
          {h!.pre}<mark>{h!.hit}</mark>{h!.post}
        {:else}{b.title}{/if}
      </a>
      {#if !edition}<span class="muted ellipsis">{authorsLine(b)}{#if b.series}{` · ${b.series.name}${b.serno ? ` #${b.serno}` : ''}`}{/if}</span>{/if}
      {#if !edition && (b.libRating || b.extRating || b.rating || (b.kidsAge !== null && b.kidsAge !== undefined))}
        <span class="rates">
          {#if b.rating}<span class="my" title={t('ratings.myTooltip')}>★ {b.rating}</span>{/if}
          {#if b.libRating}<span class="lib" title={t('ratings.libTooltip', { n: b.libRating })}>{t('ratings.libShort')} {b.libRating}/5</span>{/if}
          <ExtRating value={b.extRating} />
          <KidsBadge age={b.kidsAge} />
        </span>
      {/if}
      {#if edition}<span class="muted ellipsis">{b.series ? `${b.series.name}${b.serno ? ` #${b.serno}` : ''} · ` : ''}{b.ext.toUpperCase()} · {b.lang}</span>{/if}
      {#if w && w.others.length}
        <button type="button" class="editions" aria-expanded={expanded.has(w.key)} onclick={() => (expanded = toggleSet(expanded, w.key))}>
          {expanded.has(w.key) ? t('search.hideEditions') : tn('search.moreEditions', w.others.length)}
        </button>
      {/if}
    </div>
    <span class="muted genre">{edition ? '' : genreLine(b)}</span>
    <span class="muted date-col">{formatDate(b.date, i18nState.lang)}</span>
    <button type="button" class="send-btn" aria-label={t('selection.sendTo')} onclick={() => (send = { ids: [b.id] })}><Icon name="send" size={14} /><span>{t('selection.sendTo')}</span></button>
  </div>
{/snippet}

<div class="search-page">
  <aside class="facets" class:open={facetsOpen} aria-label={t('search.filters')}>
    <form class="search-box" role="search" onsubmit={(e) => { e.preventDefault(); submit(); }}>
      <Icon name="search" size={16} />
      <input type="search" bind:value={query} aria-label={t('search.placeholder')} placeholder={t('search.placeholder')} />
    </form>
    <button type="button" class="facets-toggle" aria-expanded={facetsOpen} onclick={() => (facetsOpen = !facetsOpen)}>
      <Icon name="filter" size={14} />{t('search.filters')}{#if filterCount}<span class="badge">{filterCount}</span>{/if}
    </button>
    {#if result && result.total + filterCount > 0}
      <div class="facet-groups">
        <div class="facet">
          <h3>{t('ratings.filtersCaps')}</h3>
          <RatingFilters filters={ratingFilters} onChange={(f) => (ratingFilters = f)} />
        </div>
        {#if result.facets.genre.length}
          <div class="facet">
            <h3>{t('search.genre')}</h3>
            {#each facetGenres as [id, count] (id)}
              <label class="fl">
                <input type="checkbox" checked={genreFilter.has(id)} onchange={() => (genreFilter = toggleSet(genreFilter, id))} />
                <span class="fname">{genresById.get(id) ?? id}</span><span class="n">{count}</span>
              </label>
            {/each}
            {#if result.facets.genre.length > 8}
              <button type="button" class="link-btn" onclick={() => (allGenres = !allGenres)}>{allGenres ? t('details.fewerSeries') : t('search.showAllGenres')}</button>
            {/if}
          </div>
        {/if}
        {#if result.facets.lang.length}
          <div class="facet">
            <h3>{t('search.language')}</h3>
            {#each result.facets.lang as [code, count] (code)}
              <label class="fl">
                <input type="checkbox" checked={langFilter.has(code)} onchange={() => (langFilter = toggleSet(langFilter, code))} />
                <span class="fname">{code}</span><span class="n">{count}</span>
              </label>
            {/each}
          </div>
        {/if}
        {#if result.facets.ext.length}
          <div class="facet">
            <h3>{t('search.format')}</h3>
            {#each result.facets.ext as [ext, count] (ext)}
              <label class="fl">
                <input type="checkbox" checked={extFilter.has(ext)} onchange={() => (extFilter = toggleSet(extFilter, ext))} />
                <span class="fname">{ext.toUpperCase()}</span><span class="n">{count}</span>
              </label>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </aside>

  <main class="results" aria-busy={loading}>
    {#if !q || q.trim().length < 2}
      <div class="empty">
        <p class="big">{t('search.startTitle')}</p>
        <p>{t('search.startHint')}</p>
      </div>
    {:else if error}
      <div class="empty"><p>{t('common.error')}: {error}</p></div>
    {:else if !result}
      <div class="empty"><p>{t('common.loading')}</p></div>
    {:else}
      <div class="summary" class:stale={loading}>
        <span><b>{result.total}</b> {tn('search.resultsCount', result.total)} «{q}»</span>
        <label class="sort-sel">
          <span class="visually-hidden">{t('books.sort')}</span>
          <select data-testid="search-sort" value={sortKey} aria-label={t('books.sort')} onchange={(e) => setPref('sort.search', (e.currentTarget as HTMLSelectElement).value)}>
            <option value="relevance">{t('search.sortRelevance')}</option>
            <option value="myRating">{t('books.sort.myRating')}</option>
            <option value="libRating">{t('books.sort.libRating')}</option>
            <option value="extRating">{t('books.sort.extRating')}</option>
          </select>
        </label>
        {#each [...genreFilter] as g (g)}
          <button type="button" class="chip" onclick={() => (genreFilter = toggleSet(genreFilter, g))}>{genresById.get(g) ?? g}<Icon name="close" size={12} /></button>
        {/each}
        {#each [...langFilter] as l (l)}
          <button type="button" class="chip" onclick={() => (langFilter = toggleSet(langFilter, l))}>{l}<Icon name="close" size={12} /></button>
        {/each}
        {#each [...extFilter] as x (x)}
          <button type="button" class="chip" onclick={() => (extFilter = toggleSet(extFilter, x))}>{x.toUpperCase()}<Icon name="close" size={12} /></button>
        {/each}
        {#if filterCount > 1}<button type="button" class="link-btn" onclick={clearFilters}>{t('books.resetFilters')}</button>{/if}
      </div>

      {#if !result.authors.length && !result.series.length && !result.books.length}
        <div class="empty">
          <p class="big">{t('search.noResults')}</p>
          <p>{filterCount ? t('search.noResultsFiltered') : t('search.noResultsHint')}</p>
          {#if filterCount}<button type="button" class="send-btn" onclick={clearFilters}>{t('books.resetFilters')}</button>{/if}
        </div>
      {/if}

      {#if result.authors.length}
        <section aria-labelledby="h-authors">
          <h2 id="h-authors">{t('search.authors')}</h2>
          <div class="series-grid">
            {#each result.authors as a (a.id)}
              <a href="/l/{lib}/authors/{a.id}" data-link class="series-card">
                <span class="name">{a.name}</span>
                <span class="muted">{tn('browse.booksCount', a.count)}</span>
              </a>
            {/each}
          </div>
        </section>
      {/if}

      {#if result.series.length}
        <section aria-labelledby="h-series">
          <h2 id="h-series">{t('search.series')}</h2>
          <div class="series-grid">
            {#each result.series as s (s.id)}
              <a href="/l/{lib}/series/{s.id}" data-link class="series-card">
                <span class="name">{s.name}</span>
                <span class="muted ellipsis">{tn('browse.booksCount', s.count)}{s.authors ? ` · ${s.authors}` : ''}</span>
              </a>
            {/each}
          </div>
        </section>
      {/if}

      {#if works.length}
        <section aria-labelledby="h-books" class="books-section">
          <h2 id="h-books">{t('search.books')}{#if result.total > result.books.length}<span class="cap">{t('search.firstN', { count: result.books.length })}</span>{/if}</h2>
          {#each works as w (w.key)}
            {@render bookRow(w.first, w, false)}
            {#if expanded.has(w.key)}
              {#each w.others as b (b.id)}{@render bookRow(b, null, true)}{/each}
            {/if}
          {/each}
        </section>
      {/if}
    {/if}
  </main>
  <DetailsPane {lib} bookId={selectedBookId} onSend={(ids, device) => (send = { ids, device })} onAddShelf={(ids) => (shelfIds = ids)} />
</div>

{#if send}<SendDialog {lib} bookIds={send.ids} device={send.device} open={true} onClose={() => (send = null)} />{/if}
{#if shelfIds}<ShelfDialog {lib} bookIds={shelfIds} open={true} onClose={() => (shelfIds = null)} />{/if}

<style>
  .search-page { display: flex; flex-grow: 1; min-width: 0; min-height: 0; overflow: hidden; }
  .facets { width: 240px; flex-shrink: 0; border-right: 1px solid var(--line); padding: 16px 16px 20px; display: flex; flex-direction: column; gap: 18px; overflow-y: auto; background: var(--surface-alt); }
  .facet-groups { display: flex; flex-direction: column; gap: 18px; }
  .search-box { display: flex; align-items: center; gap: 8px; height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); color: var(--muted); flex-shrink: 0; }
  .search-box:focus-within { border-color: var(--accent); }
  .search-box :global(svg) { flex-shrink: 0; }
  .search-box input { border: none; outline: none; background: transparent; flex-grow: 1; min-width: 0; font: inherit; font-size: 14px; color: var(--ink); }
  .facets-toggle { display: none; }
  h3 { margin: 0 0 6px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; }
  .fl { display: flex; align-items: center; gap: 8px; font-size: 13px; padding: 3px 0; min-width: 0; }
  .fl input { width: 15px; height: 15px; accent-color: var(--accent); flex-shrink: 0; }
  .fname { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .n { margin-left: auto; color: var(--muted); font-size: 12px; flex-shrink: 0; font-variant-numeric: tabular-nums; }
  .link-btn { all: unset; color: var(--accent); font-size: 12px; cursor: pointer; margin-top: 4px; }
  .link-btn:hover { text-decoration: underline; }
  .results { flex: 1 1 0; min-width: 0; overflow-y: auto; padding: 20px 28px; display: flex; flex-direction: column; gap: 22px; }
  .empty { flex-grow: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; color: var(--muted); text-align: center; }
  .empty p { margin: 0; max-width: 460px; }
  .empty .big { font-family: var(--font-display); font-size: 20px; color: var(--ink); }
  .summary { display: flex; align-items: center; gap: 8px; font-size: 14px; color: var(--muted); flex-wrap: wrap; }
  .summary.stale { opacity: .6; }
  .sort-sel select { height: 28px; padding: 0 6px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); color: var(--muted-2); font: inherit; font-size: 13px; }
  .rates { display: inline-flex; align-items: center; gap: 8px; font-size: 12px; color: var(--muted); }
  .rates .my { color: var(--amber); }
  .chip { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border-radius: 13px; background: var(--accent-soft); color: var(--accent-soft-ink); font-size: 12px; border: none; }
  h2 { margin: 0 0 10px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; display: flex; gap: 10px; align-items: baseline; }
  .cap { font-weight: 400; letter-spacing: 0; }
  .series-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 10px; }
  .series-card { display: flex; flex-direction: column; gap: 4px; padding: 12px 14px; border-radius: 10px; background: var(--surface); border: 1px solid var(--line); color: var(--ink); text-decoration: none; min-width: 0; }
  .series-card:hover { border-color: var(--accent); text-decoration: none; }
  .series-card .name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .muted { color: var(--muted); font-size: 13px; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .books-section { background: var(--surface); border: 1px solid var(--line); border-radius: 10px; overflow: hidden; flex-shrink: 0; }
  .books-section h2 { margin: 0; padding: 12px 16px; border-bottom: 1px solid var(--line-soft); }
  .res-row { display: grid; grid-template-columns: 44px minmax(0,1fr) minmax(0, 150px) 90px auto; align-items: center; gap: 14px; padding: 10px 16px; border-bottom: 1px solid var(--line-soft); }
  .res-row.selected { background: var(--accent-soft); }
  .res-row.edition { padding-top: 6px; padding-bottom: 6px; background: var(--surface-alt); }
  .res-row.edition .title { font-size: 14px; font-weight: 400; font-family: var(--font-body); }
  .res-row:last-child { border-bottom: none; }
  .info { display: flex; flex-direction: column; gap: 3px; min-width: 0; align-items: flex-start; }
  .info > * { max-width: 100%; }
  .title { font-family: var(--font-display); font-size: 16px; font-weight: 600; color: var(--ink); text-decoration: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .title:hover { color: var(--accent); }
  .editions { all: unset; font-size: 12px; color: var(--accent); cursor: pointer; }
  .editions:hover { text-decoration: underline; }
  .editions:focus-visible { outline: 2px solid var(--focus); }
  mark { background: rgba(184,116,26,.25); color: inherit; border-radius: 2px; }
  .genre { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .date-col { font-variant-numeric: tabular-nums; }
  .send-btn { justify-self: end; display: inline-flex; align-items: center; gap: 6px; height: 34px; padding: 0 12px; border-radius: 7px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 13px; white-space: nowrap; flex-shrink: 0; }
  .send-btn:hover { background: var(--surface-hover); }
  .send-btn :global(svg) { flex-shrink: 0; }
  .badge { min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--accent); color: #fff; font-size: 11px; display: inline-flex; align-items: center; justify-content: center; }

  @media (max-width: 1280px) {
    .res-row { grid-template-columns: 44px minmax(0,1fr) 90px auto; }
    .res-row .genre { display: none; }
    .send-btn span { display: none; }
  }
  @media (max-width: 900px) {
    .search-page { flex-direction: column; overflow-y: auto; overflow-x: hidden; }
    .facets { width: auto; border-right: none; border-bottom: 1px solid var(--line); padding: 10px 12px; gap: 10px; overflow: visible; flex-direction: row; flex-wrap: wrap; align-items: center; }
    .facets .search-box { flex: 1 1 200px; }
    .facets-toggle { display: inline-flex; align-items: center; gap: 6px; height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font-size: 13px; color: var(--muted-2); }
    .facet-groups { display: none; flex-basis: 100%; flex-direction: row; flex-wrap: wrap; gap: 12px 24px; }
    .facets.open .facet-groups { display: flex; }
    .results { padding: 14px 12px 24px; gap: 16px; overflow: visible; }
    .series-grid { grid-template-columns: 1fr; }
    .res-row { grid-template-columns: 40px minmax(0,1fr) auto; gap: 10px; padding: 10px 12px; }
    .res-row .genre, .res-row .date-col { display: none; }
  }
</style>
