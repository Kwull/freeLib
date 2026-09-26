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
  import Hl from '../components/Hl.svelte';
  import EditionsList from '../components/EditionsList.svelte';
  import { formatDate } from '../utils/format';
  import RatingFilters from '../components/RatingFilters.svelte';
  import ExtRating from '../components/ExtRating.svelte';
  import KidsBadge from '../components/KidsBadge.svelte';
  import { getPref, setPref } from '../stores/prefs.svelte';
  import {
    emptyRatingFilters, ratingFilterCount, ratingParams, type RatingFilters as RatingFiltersT, type RatingSortKey,
  } from '../utils/ratings';

  let { lib, q, exact = false }: { lib: number; q: string; exact?: boolean } = $props();

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
  let expanded = $state<Set<number>>(new Set());
  const groupEditions = $derived(getPref<boolean>('groupEditions', true));
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
      group: groupEditions,
      exact,
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

  // words to highlight: the server says which words of the shown names matched (by prefix,
  // word form, transliteration or a typo fix)
  const hl = $derived(new Set(result?.highlight ?? []));
  const searchFor = (text: string) => `/l/${lib}/search?q=${encodeURIComponent(text)}`;

  function pick(b: Book) { pickId(b.id); }
  function pickId(id: number) {
    if (window.innerWidth < 900) navigate(`/l/${lib}/book/${id}`);
    else selectedBookId = id;
  }
  const selectedId = $derived(selectedBookId);
  const authorsLine = (b: Book) =>
    b.authors.length > 3 ? `${b.authors.slice(0, 2).map((a) => a.name).join(', ')} ${t('books.andMore', { count: b.authors.length - 2 })}` : b.authors.map((a) => a.name).join(', ');
  const genreLine = (b: Book) => b.genres.map((g) => genresById.get(g)).filter(Boolean)[0] ?? '';
  const facetGenres = $derived(result ? (allGenres ? result.facets.genre : result.facets.genre.slice(0, 8)) : []);
</script>

{#snippet bookRow(b: Book)}
  <div class="res-row" class:selected={b.id === selectedBookId} data-testid="search-book">
    <CoverThumb {lib} bookId={b.id} title={b.title} />
    <div class="info">
      <a href="/l/{lib}/book/{b.id}" class="title" onclick={(e) => { e.preventDefault(); pick(b); }}><Hl text={b.title} words={hl} /></a>
      <span class="muted byline"><Hl text={authorsLine(b)} words={hl} />{#if b.series}{' · '}<Hl text={`${b.series.name}${b.serno ? ` #${b.serno}` : ''}`} words={hl} />{/if}</span>
      <span class="meta">
        {#if genreLine(b)}<span class="genre">{genreLine(b)}</span>{/if}
        <span class="date">{formatDate(b.date, i18nState.lang)}</span>
        {#if b.rating}<span class="my" title={t('ratings.myTooltip')}>★ {b.rating}</span>{/if}
        {#if b.libRating}<span class="lib" title={t('ratings.libTooltip', { n: b.libRating })}>{t('ratings.libShort')} {b.libRating}/5</span>{/if}
        <ExtRating value={b.extRating} />
        <KidsBadge age={b.kidsAge} />
        {#if b.editions}
          <button type="button" class="editions" data-testid="editions-toggle" aria-expanded={expanded.has(b.id)} onclick={() => (expanded = toggleSet(expanded, b.id))}>
            <Icon name="layers" size={12} />{expanded.has(b.id) ? t('search.hideEditions') : tn('editions.countBest', b.editions.count)}
          </button>
        {/if}
      </span>
    </div>
    <button type="button" class="send-btn" aria-label={t('selection.sendTo')} title={t('selection.sendTo')} onclick={() => (send = { ids: [b.id] })}><Icon name="send" size={14} /><span>{t('selection.sendTo')}</span></button>
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
        <span><b>{result.total}</b> {tn('search.resultsCount', result.total)} «{result.corrected ?? q}»</span>
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

      {#if result.corrected}
        <p class="dym" data-testid="search-corrected">
          {t('search.correctedTo')} <b>{result.corrected}</b> · {t('search.insteadFor')} <a href="{searchFor(q)}&exact=1" data-link data-testid="search-exact">{q}</a>
        </p>
      {:else if result.didYouMean}
        <p class="dym" data-testid="did-you-mean">
          {t('search.didYouMean')} <a href={searchFor(result.didYouMean)} data-link>{result.didYouMean}</a>?
        </p>
      {/if}

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
                <span class="name"><Hl text={a.name} words={hl} /></span>
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
                <span class="name"><Hl text={s.name} words={hl} /></span>
                <span class="muted ellipsis">{tn('browse.booksCount', s.count)}{s.authors ? ` · ${s.authors}` : ''}</span>
              </a>
            {/each}
          </div>
        </section>
      {/if}

      {#if result.books.length}
        <section aria-labelledby="h-books" class="books-section">
          <h2 id="h-books">{t('search.books')}{#if result.total > result.books.length}<span class="cap">{t('search.firstN', { count: result.books.length })}</span>{/if}
            <label class="group-toggle"><input type="checkbox" data-testid="group-editions" checked={groupEditions} onchange={() => setPref('groupEditions', !groupEditions)} />{t('editions.group')}</label>
          </h2>
          {#each result.books as b (b.id)}
            {@render bookRow(b)}
            {#if expanded.has(b.id)}
              <EditionsList {lib} bookId={b.id} {selectedId} onPick={(id) => pickId(id)} onSend={(ids) => (send = { ids })} />
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
  .meta { display: flex; align-items: center; flex-wrap: wrap; gap: 4px 10px; font-size: 12px; color: var(--muted); }
  .meta .my { color: var(--amber); }
  .meta .date { font-variant-numeric: tabular-nums; }
  .meta .genre { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 220px; }
  .byline { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .facets :global(.rf) { min-width: 0; }
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
  .res-row { display: grid; grid-template-columns: 44px minmax(0,1fr) auto; align-items: center; gap: 14px; padding: 10px 16px; border-bottom: 1px solid var(--line-soft); }
  .res-row.selected { background: var(--accent-soft); }
  .res-row:last-child { border-bottom: none; }
  .info { display: flex; flex-direction: column; gap: 3px; min-width: 0; align-items: flex-start; }
  .info > * { max-width: 100%; }
  .title { font-family: var(--font-display); font-size: 16px; font-weight: 600; color: var(--ink); text-decoration: none; overflow: hidden; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow-wrap: anywhere; }
  .title:hover { color: var(--accent); }
  .editions { all: unset; display: inline-flex; align-items: center; gap: 4px; font-size: 12px; color: var(--accent); cursor: pointer; white-space: nowrap; }
  .dym { margin: 0; font-size: 15px; color: var(--ink); }
  .dym a { font-weight: 600; }
  .group-toggle { margin-left: auto; display: inline-flex; align-items: center; gap: 6px; font-weight: 400; letter-spacing: 0; font-size: 12px; color: var(--muted-2); text-transform: none; }
  .group-toggle input { accent-color: var(--accent); }
  .editions:hover { text-decoration: underline; }
  .editions:focus-visible { outline: 2px solid var(--focus); }
  .send-btn { justify-self: end; display: inline-flex; align-items: center; gap: 6px; height: 34px; padding: 0 12px; border-radius: 7px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 13px; white-space: nowrap; flex-shrink: 0; }
  .send-btn:hover { background: var(--surface-hover); }
  .send-btn :global(svg) { flex-shrink: 0; }
  .badge { min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--accent); color: #fff; font-size: 11px; display: inline-flex; align-items: center; justify-content: center; }

  /* the results column narrows with the details pane open: icon-only send, no genre */
  .results { container-type: inline-size; }
  @container (max-width: 720px) {
    .send-btn { padding: 0 10px; }
    .send-btn span { display: none; }
    .meta .genre { display: none; }
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
  }
</style>
