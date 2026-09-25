<script lang="ts">
  import { api } from '../api/client';
  import type { SearchResponse } from '../api/types';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';
  import Icon from '../components/Icon.svelte';
  import CoverThumb from '../components/CoverThumb.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import { normalize } from '../utils/normalize';

  let { lib, q }: { lib: number; q: string } = $props();

  let query = $state('');
  let result = $state<SearchResponse | null>(null);
  let genreFilter = $state<Set<number>>(new Set());
  let langFilter = $state<Set<string>>(new Set());
  let extFilter = $state<Set<string>>(new Set());
  let sendIds = $state<number[] | null>(null);
  let genresById = $state<Map<number, string>>(new Map());

  $effect(() => { query = q; });
  $effect(() => {
    api.genres(lib).then((gs) => (genresById = new Map(gs.map((g) => [g.id, g.name]))));
  });

  $effect(() => {
    if (!q || q.trim().length < 2) { result = null; return; }
    let cancelled = false;
    api.search(lib, {
      q,
      kind: 'all',
      genre: [...genreFilter].join(',') || undefined,
      lang: [...langFilter].join(',') || undefined,
      ext: [...extFilter].join(',') || undefined,
    }).then((r) => { if (!cancelled) result = r; });
    return () => { cancelled = true; };
  });

  function submit() {
    navigate(`/l/${lib}/search?q=${encodeURIComponent(query)}`);
  }
  function toggleSet(set: Set<number | string>, v: number | string) {
    const next = new Set(set);
    if (next.has(v)) next.delete(v); else next.add(v);
    return next;
  }

  function highlight(title: string): { pre: string; hit: string; post: string } | null {
    const words = normalize(q).split(' ').filter(Boolean);
    const norm = normalize(title);
    for (const w of words) {
      const idx = norm.indexOf(w);
      if (idx >= 0) return { pre: title.slice(0, idx), hit: title.slice(idx, idx + w.length), post: title.slice(idx + w.length) };
    }
    return null;
  }
</script>

<div class="search-page">
  <aside class="facets" aria-label="Filters">
    <div class="search-box">
      <Icon name="search" size={16} />
      <input type="text" bind:value={query} onkeydown={(e) => e.key === 'Enter' && submit()} aria-label={t('search.placeholder')} />
    </div>
    {#if result}
      <div class="facet">
        <h3>{t('search.genre')}</h3>
        {#each result.facets.genre.slice(0, 8) as [id, count] (id)}
          <label class="fl">
            <input type="checkbox" checked={genreFilter.has(id)} onchange={() => (genreFilter = toggleSet(genreFilter, id) as Set<number>)} />
            {genresById.get(id) ?? id}<span class="n">{count}</span>
          </label>
        {/each}
      </div>
      <div class="facet">
        <h3>{t('search.language')}</h3>
        {#each result.facets.lang as [code, count] (code)}
          <label class="fl">
            <input type="checkbox" checked={langFilter.has(code)} onchange={() => (langFilter = toggleSet(langFilter, code) as Set<string>)} />
            {code}<span class="n">{count}</span>
          </label>
        {/each}
      </div>
      <div class="facet">
        <h3>{t('search.format')}</h3>
        {#each result.facets.ext as [ext, count] (ext)}
          <label class="fl">
            <input type="checkbox" checked={extFilter.has(ext)} onchange={() => (extFilter = toggleSet(extFilter, ext) as Set<string>)} />
            {ext.toUpperCase()}<span class="n">{count}</span>
          </label>
        {/each}
      </div>
    {/if}
  </aside>

  <main class="results">
    {#if !result}
      <p class="empty">{t('search.noResults')}</p>
    {:else}
      <div class="summary">
        <span><b>{result.total}</b> {t('search.results')} «{q}»</span>
        {#each [...genreFilter] as g (g)}
          <button type="button" class="chip" onclick={() => (genreFilter = toggleSet(genreFilter, g) as Set<number>)}>{genresById.get(g) ?? g}<Icon name="close" size={12} /></button>
        {/each}
        {#each [...langFilter] as l (l)}
          <button type="button" class="chip" onclick={() => (langFilter = toggleSet(langFilter, l) as Set<string>)}>{l}<Icon name="close" size={12} /></button>
        {/each}
      </div>

      {#if result.authors.length}
        <section aria-labelledby="h-authors">
          <h2 id="h-authors">{t('search.authors')}</h2>
          <div class="series-grid">
            {#each result.authors as a (a.id)}
              <a href="/l/{lib}/authors/{a.id}" data-link class="series-card">
                <span class="name">{a.name}</span>
                <span class="muted">{t('browse.booksCount', { count: a.count })}</span>
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
                <span class="muted">{s.count} · {s.authors}</span>
              </a>
            {/each}
          </div>
        </section>
      {/if}

      {#if result.books.length}
        <section aria-labelledby="h-books" class="books-section">
          <h2 id="h-books">{t('search.books')}</h2>
          {#each result.books as b (b.id)}
            <div class="res-row">
              <CoverThumb {lib} bookId={b.id} title={b.title} />
              <div class="info">
                <a href="/l/{lib}/book/{b.id}" data-link class="title">
                  {#if highlight(b.title)}
                    {@const h = highlight(b.title)}
                    {h!.pre}<mark>{h!.hit}</mark>{h!.post}
                  {:else}{b.title}{/if}
                </a>
                <span class="muted">{b.authors.map((a) => a.name).join(', ')}</span>
              </div>
              <span class="muted genre">{b.genres.map((g) => genresById.get(g)).filter(Boolean)[0] ?? ''}</span>
              <span class="muted">{b.date}</span>
              <button type="button" class="send-btn" onclick={() => (sendIds = [b.id])}><Icon name="send" size={14} />{t('selection.sendTo')}</button>
            </div>
          {/each}
        </section>
      {/if}
    {/if}
  </main>
</div>

{#if sendIds}<SendDialog {lib} bookIds={sendIds} open={true} onClose={() => (sendIds = null)} />{/if}

<style>
  .search-page { display: flex; flex-grow: 1; min-width: 0; min-height: 0; overflow: hidden; }
  .facets { width: 240px; flex-shrink: 0; border-right: 1px solid var(--line); padding: 20px 20px; display: flex; flex-direction: column; gap: 20px; overflow-y: auto; background: var(--surface-alt); }
  .search-box { display: flex; align-items: center; gap: 8px; height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); }
  .search-box input { border: none; outline: none; background: transparent; flex-grow: 1; font: inherit; font-size: 14px; }
  h3 { margin: 0 0 8px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; }
  .fl { display: flex; align-items: center; gap: 8px; font-size: 13px; padding: 4px 0; }
  .fl input { width: 15px; height: 15px; accent-color: var(--accent); }
  .n { margin-left: auto; color: var(--muted); font-size: 12px; }
  .results { flex-grow: 1; min-width: 0; overflow-y: auto; padding: 20px 28px; display: flex; flex-direction: column; gap: 22px; }
  .empty { color: var(--muted); }
  .summary { display: flex; align-items: center; gap: 8px; font-size: 14px; color: var(--muted); flex-wrap: wrap; }
  .chip { display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border-radius: 13px; background: var(--accent-soft); color: var(--accent-soft-ink); font-size: 12px; border: none; }
  h2 { margin: 0 0 10px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; }
  .series-grid { display: grid; grid-template-columns: repeat(3, minmax(0,1fr)); gap: 12px; }
  .series-card { display: flex; flex-direction: column; gap: 4px; padding: 14px 16px; border-radius: 10px; background: var(--surface); border: 1px solid var(--line); color: var(--ink); text-decoration: none; }
  .series-card .name { font-weight: 600; }
  .muted { color: var(--muted); font-size: 13px; }
  .books-section { background: var(--surface); border: 1px solid var(--line); border-radius: 10px; overflow: hidden; }
  .books-section h2 { margin: 0; padding: 12px 16px; border-bottom: 1px solid var(--line-soft); }
  .res-row { display: grid; grid-template-columns: 44px minmax(0,1fr) 160px 100px 100px; align-items: center; gap: 14px; padding: 10px 16px; border-bottom: 1px solid var(--line-soft); }
  .res-row:last-child { border-bottom: none; }
  .info { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .title { font-family: var(--font-display); font-size: 16px; font-weight: 600; color: var(--ink); text-decoration: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  mark { background: rgba(184,116,26,.25); color: inherit; border-radius: 2px; }
  .genre { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .send-btn { justify-self: end; display: inline-flex; align-items: center; gap: 6px; height: 34px; padding: 0 12px; border-radius: 7px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 13px; }
</style>
