<script lang="ts">
  import { api } from '../api/client';
  import type { SearchResponse } from '../api/types';
  import { debounce } from '../utils/format';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';

  let { lib }: { lib: number } = $props();

  let inputEl: HTMLInputElement | undefined;
  let query = $state('');
  let open = $state(false);
  let result = $state<SearchResponse | null>(null);

  const runSearch = debounce(async (q: string) => {
    if (q.trim().length < 2) { result = null; return; }
    result = await api.search(lib, { q, kind: 'all', limit: 5 });
  }, 180);

  $effect(() => { runSearch(query); });

  function submit() {
    if (query.trim().length >= 2) navigate(`/l/${lib}/search?q=${encodeURIComponent(query)}`);
    open = false;
  }

  function globalKeydown(e: KeyboardEvent) {
    if (e.key === '/' && document.activeElement !== inputEl && !(document.activeElement instanceof HTMLInputElement)) {
      e.preventDefault();
      inputEl?.focus();
    }
  }
  $effect(() => {
    window.addEventListener('keydown', globalKeydown);
    return () => window.removeEventListener('keydown', globalKeydown);
  });
</script>

<div class="wrap">
  <label class="box" class:focused={open}>
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>
    <input
      bind:this={inputEl}
      type="text"
      placeholder={t('search.placeholder')}
      bind:value={query}
      onfocus={() => (open = true)}
      onblur={() => setTimeout(() => (open = false), 150)}
      onkeydown={(e) => e.key === 'Enter' && submit()}
    />
    <kbd>/</kbd>
  </label>
  {#if open && result && (result.authors.length || result.series.length || result.books.length)}
    <div class="dropdown" role="listbox">
      {#if result.authors.length}
        <div class="group-label">{t('search.authors')}</div>
        {#each result.authors.slice(0, 4) as a (a.id)}
          <a href="/l/{lib}/authors/{a.id}" data-link class="row">{a.name}<span class="n">{a.count}</span></a>
        {/each}
      {/if}
      {#if result.series.length}
        <div class="group-label">{t('search.series')}</div>
        {#each result.series.slice(0, 4) as s (s.id)}
          <a href="/l/{lib}/series/{s.id}" data-link class="row">{s.name}<span class="n">{s.count}</span></a>
        {/each}
      {/if}
      {#if result.books.length}
        <div class="group-label">{t('search.books')}</div>
        {#each result.books.slice(0, 4) as b (b.id)}
          <a href="/l/{lib}/book/{b.id}" data-link class="row">{b.title}</a>
        {/each}
      {/if}
      <button type="button" class="all" onclick={submit}>{t('search.results')} «{query}» →</button>
    </div>
  {/if}
</div>

<style>
  .wrap { position: relative; flex-grow: 1; max-width: 640px; }
  .box {
    display: flex; align-items: center; gap: 10px; height: 38px; padding: 0 14px; border-radius: 8px;
    background: var(--surface-hover); color: var(--muted);
  }
  .box.focused { outline: 2px solid var(--focus); }
  input { flex-grow: 1; border: none; outline: none; background: transparent; font: inherit; font-size: 14px; color: var(--ink); }
  kbd { font: inherit; font-size: 12px; padding: 2px 6px; border: 1px solid var(--border); border-radius: 4px; background: var(--page); color: var(--muted); }
  .dropdown {
    position: absolute; top: 44px; left: 0; right: 0; background: var(--surface); border: 1px solid var(--line);
    border-radius: 10px; box-shadow: 0 12px 32px rgba(0,0,0,.18); padding: 8px; z-index: 50; max-height: 60vh; overflow-y: auto;
  }
  .group-label { padding: 8px 10px 4px; font-size: 11px; font-weight: 600; color: var(--muted); letter-spacing: .04em; }
  .row { display: flex; justify-content: space-between; gap: 10px; padding: 8px 10px; border-radius: 6px; font-size: 14px; color: var(--ink); text-decoration: none; }
  .row:hover { background: var(--surface-hover); text-decoration: none; }
  .n { color: var(--muted); font-size: 12px; }
  .all { all: unset; display: block; width: 100%; box-sizing: border-box; padding: 10px; text-align: center; color: var(--accent); font-size: 13px; cursor: pointer; border-top: 1px solid var(--line-soft); margin-top: 4px; }
</style>
