<script lang="ts">
  import { api } from '../api/client';
  import type { SearchResponse } from '../api/types';
  import { navigate } from '../router.svelte';
  import { t, tn } from '../i18n';
  import Hl from './Hl.svelte';

  let { lib }: { lib: number } = $props();

  let inputEl: HTMLInputElement | undefined;
  let query = $state('');
  let open = $state(false);
  let result = $state<SearchResponse | null>(null);
  let active = $state(-1);
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    const q = query;
    clearTimeout(timer);
    active = -1;
    if (q.trim().length < 2) { result = null; return; }
    timer = setTimeout(async () => {
      const mine = ++seq;
      try {
        const r = await api.search(lib, { q, kind: 'all', limit: 5, group: true });
        if (mine === seq) result = r; // ignore late answers to older keystrokes
      } catch { /* the results page shows errors */ }
    }, 160);
  });

  type Item = { href: string; label: string; sub?: string; n?: string; group: string };
  const items = $derived.by<Item[]>(() => {
    if (!result) return [];
    const out: Item[] = [];
    const fix = result.corrected ?? result.didYouMean;
    if (fix) out.push({ group: 'fix', href: `/l/${lib}/search?q=${encodeURIComponent(fix)}`, label: fix });
    for (const a of result.authors.slice(0, 4)) out.push({ group: 'authors', href: `/l/${lib}/authors/${a.id}`, label: a.name, n: String(a.count) });
    for (const s of result.series.slice(0, 3)) out.push({ group: 'series', href: `/l/${lib}/series/${s.id}`, label: s.name, sub: s.authors, n: String(s.count) });
    for (const b of result.books.slice(0, 5)) {
      const author = b.authors[0];
      // desktop: open the book in its author's list; phone: the book page
      const href = author && window.innerWidth >= 900
        ? `/l/${lib}/authors/${author.id}?book=${b.id}`
        : `/l/${lib}/book/${b.id}`;
      const names = b.authors.length > 2 ? `${b.authors[0].name} ${t('books.andMore', { count: b.authors.length - 1 })}` : b.authors.map((a) => a.name).join(', ');
      out.push({ group: 'books', href, label: b.title, sub: names });
    }
    return out;
  });

  function go(href: string) {
    open = false;
    inputEl?.blur();
    navigate(href);
  }

  function submit() {
    if (active >= 0 && items[active]) { go(items[active].href); return; }
    if (query.trim().length >= 2) go(`/l/${lib}/search?q=${encodeURIComponent(query.trim())}`);
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') { e.preventDefault(); open = true; active = Math.min(items.length - 1, active + 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); active = Math.max(-1, active - 1); }
    else if (e.key === 'Enter') { e.preventDefault(); submit(); }
    else if (e.key === 'Escape') { if (open) { open = false; } else { query = ''; inputEl?.blur(); } }
  }

  function globalKeydown(e: KeyboardEvent) {
    const el = document.activeElement;
    const typing = el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement || (el as HTMLElement)?.isContentEditable;
    if (e.key === '/' && !typing && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      inputEl?.focus();
      inputEl?.select();
    }
  }
  $effect(() => {
    window.addEventListener('keydown', globalKeydown);
    return () => window.removeEventListener('keydown', globalKeydown);
  });

  const groupLabel: Record<string, string> = $derived({
    authors: t('search.authors'), series: t('search.series'), books: t('search.books'),
    fix: result?.corrected ? t('search.showingFor') : t('search.didYouMeanCaps'),
  });
  const hl = $derived(new Set(result?.highlight ?? []));
</script>

<div class="wrap">
  <label class="box" class:focused={open}>
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>
    <input
      bind:this={inputEl}
      type="text"
      role="combobox"
      aria-expanded={open && items.length > 0}
      aria-controls="global-search-list"
      aria-activedescendant={active >= 0 ? `gs-${active}` : undefined}
      aria-autocomplete="list"
      placeholder={t('search.placeholder')}
      bind:value={query}
      onfocus={() => (open = true)}
      oninput={() => (open = true)}
      onblur={() => setTimeout(() => (open = false), 150)}
      onkeydown={keydown}
    />
    <kbd>/</kbd>
  </label>
  {#if open && query.trim().length >= 2 && result}
    <div class="dropdown" id="global-search-list" role="listbox">
      {#each items as it, i (it.href)}
        {#if i === 0 || items[i - 1].group !== it.group}<div class="group-label">{groupLabel[it.group]}</div>{/if}
        <a
          id="gs-{i}"
          role="option"
          aria-selected={i === active}
          href={it.href}
          class="row"
          class:active={i === active}
          onmousedown={(e) => { e.preventDefault(); go(it.href); }}
        >
          <span class="lbl"><span class="main" data-testid={it.group === 'fix' ? 'typeahead-fix' : undefined}><Hl text={it.label} words={it.group === 'fix' ? null : hl} /></span>{#if it.sub}<span class="sub"><Hl text={it.sub} words={hl} /></span>{/if}</span>
          {#if it.n}<span class="n">{it.n}</span>{/if}
        </a>
      {/each}
      {#if !items.length}<div class="none">{t('search.noResults')}</div>{/if}
      <button type="button" class="all" onmousedown={(e) => { e.preventDefault(); active = -1; submit(); }}>
        {result.total ? tn('search.allResults', result.total) : t('search.results')} «{query.trim()}» →
      </button>
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
  input { flex-grow: 1; min-width: 0; border: none; outline: none; background: transparent; font: inherit; font-size: 14px; color: var(--ink); }
  kbd { font: inherit; font-size: 12px; padding: 2px 6px; border: 1px solid var(--border); border-radius: 4px; background: var(--page); color: var(--muted); }
  .dropdown {
    position: absolute; top: 44px; left: 0; right: 0; background: var(--surface); border: 1px solid var(--line);
    border-radius: 10px; box-shadow: 0 12px 32px rgba(0,0,0,.18); padding: 8px; z-index: 50; max-height: 70vh; overflow-y: auto;
  }
  .group-label { padding: 8px 10px 4px; font-size: 11px; font-weight: 600; color: var(--muted); letter-spacing: .04em; }
  .row { display: flex; justify-content: space-between; align-items: center; gap: 10px; padding: 6px 10px; border-radius: 6px; font-size: 14px; color: var(--ink); text-decoration: none; }
  .row:hover, .row.active { background: var(--surface-hover); text-decoration: none; color: var(--ink); }
  .lbl { display: flex; flex-direction: column; min-width: 0; }
  .main, .sub { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sub { font-size: 12px; color: var(--muted); }
  .n { color: var(--muted); font-size: 12px; flex-shrink: 0; }
  .none { padding: 10px; color: var(--muted); font-size: 13px; }
  .all { all: unset; display: block; width: 100%; box-sizing: border-box; padding: 10px; text-align: center; color: var(--accent); font-size: 13px; cursor: pointer; border-top: 1px solid var(--line-soft); margin-top: 4px; }
  .all:hover { background: var(--surface-hover); }
</style>
