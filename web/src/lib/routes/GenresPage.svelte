<script lang="ts">
  import { api } from '../api/client';
  import type { Genre } from '../api/types';
  import BooksPane from '../components/BooksPane.svelte';
  import DetailsPane from '../components/DetailsPane.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import ShelfDialog from '../components/ShelfDialog.svelte';
  import Splitter from '../components/Splitter.svelte';
  import { navigate } from '../router.svelte';
  import { t, i18nState } from '../i18n';
  import { PANE_LIMITS, paneWidth, setPaneWidth } from '../stores/layout.svelte';

  let { lib, id }: { lib: number; id: number | null } = $props();

  let genres = $state<Genre[]>([]);
  let selectedBookId = $state<number | null>(null);
  let send = $state<{ ids: number[]; device?: number } | null>(null);
  let liveWidth = $state<number | null>(null);
  const treeWidth = $derived(liveWidth ?? paneWidth('list'));
  let shelfIds = $state<number[] | null>(null);

  $effect(() => { const lang = i18nState.lang; api.genres(lib, lang).then((g) => (genres = g)).catch(() => {}); });
  $effect(() => { selectedBookId = null; });

  let genreQuery = $state('');
  let hideEmpty = $state(true);
  let openGroups = $state<Set<number>>(new Set());
  const filtering = $derived(genreQuery.trim().length > 0);
  const matches = (g: Genre) => g.name.toLowerCase().includes(genreQuery.trim().toLowerCase());
  const top = $derived(genres.filter((g) => g.parent === 0 && (!hideEmpty || g.count > 0)));
  const childrenOf = (pid: number) =>
    genres.filter((g) => g.parent === pid && (!hideEmpty || g.count > 0) && (!filtering || matches(g)));
  // groups start folded (322 genres are a long list); the current genre's group is open
  const isOpen = (gid: number) => filtering || openGroups.has(gid) || (current !== null && (current.id === gid || current.parent === gid));
  function toggleOpen(gid: number) {
    const s = new Set(openGroups);
    if (isOpen(gid) && s.has(gid)) s.delete(gid); else s.add(gid);
    openGroups = s;
  }
  const current = $derived(genres.find((g) => g.id === id) ?? null);
  let mobilePane = $state<'list' | 'books'>('list');
  $effect(() => { mobilePane = id ? 'books' : 'list'; });

  let liveBooksCount = $state(0);
  $effect(() => { liveBooksCount = current ? current.count : 0; });

  const parentName = $derived(current ? genres.find((g) => g.id === current.parent)?.name : undefined);
  const header = $derived(
    current
      ? {
          crumb: `${t('nav.genres')}${parentName ? ` / ${parentName}` : ''}`,
          name: current.name,
          booksCount: liveBooksCount,
        }
      : undefined,
  );

  function pickBook(bid: number) {
    selectedBookId = bid;
    if (window.innerWidth < 900) navigate(`/l/${lib}/book/${bid}`);
  }
</script>

<div class="genres-page" class:show-list={mobilePane === 'list'} class:show-books={mobilePane === 'books'}>
  <section aria-label={t('genres.title')} class="tree" style:--w="{treeWidth}px">
    <div class="head">
      <h2>{t('genres.title')}</h2>
      <label class="filter-box">
        <input type="text" bind:value={genreQuery} placeholder={t('genres.filter')} aria-label={t('genres.filter')} />
      </label>
      <label class="hide-empty"><input type="checkbox" bind:checked={hideEmpty} />{t('genres.hideEmpty')}</label>
    </div>
    <div class="scroll">
      {#each top as g (g.id)}
        {@const kids = childrenOf(g.id)}
        {#if !filtering || kids.length || matches(g)}
          <div class="parent-row" class:active={g.id === id}>
            <button type="button" class="exp" aria-label={g.name} aria-expanded={isOpen(g.id)} onclick={() => toggleOpen(g.id)}>
              <span class="chev" class:open={isOpen(g.id)}>›</span>
            </button>
            <a href="/l/{lib}/genres/{g.id}" data-link class="row parent" class:active={g.id === id} class:zero={g.count === 0}>{g.name}<span class="n">{g.count}</span></a>
          </div>
          {#if isOpen(g.id)}
            {#each kids as c (c.id)}
              <a href="/l/{lib}/genres/{c.id}" data-link class="row child" class:active={c.id === id} class:zero={c.count === 0}>{c.name}<span class="n">{c.count}</span></a>
            {/each}
          {/if}
        {/if}
      {/each}
    </div>
  </section>
  <Splitter
    value={treeWidth}
    min={PANE_LIMITS.list.min}
    max={PANE_LIMITS.list.max}
    label={t('layout.resizeList')}
    onInput={(v) => (liveWidth = v)}
    onCommit={(v) => { setPaneWidth('list', v); liveWidth = null; }}
    onReset={() => { setPaneWidth('list', null); liveWidth = null; }}
  />

  {#if id !== null && current}
    <BooksPane
      {lib}
      scope={{ kind: 'genre', id, groupable: false }}
      {selectedBookId}
      onPick={pickBook}
      onOpenSend={(ids) => (send = { ids })}
      onOpenShelf={(ids) => (shelfIds = ids)}
      {header}
      onBack={() => (mobilePane = 'list')}
      onCounts={(c) => (liveBooksCount = c.books)}
    />
    <DetailsPane {lib} bookId={selectedBookId} onSend={(ids, device) => (send = { ids, device })} onAddShelf={(ids) => (shelfIds = ids)} />
  {:else}
    <div class="placeholder">{t('genres.pick')}</div>
  {/if}
</div>

{#if send}<SendDialog {lib} bookIds={send.ids} device={send.device} open={true} onClose={() => (send = null)} />{/if}
{#if shelfIds}<ShelfDialog {lib} bookIds={shelfIds} open={true} onClose={() => (shelfIds = null)} />{/if}

<style>
  .genres-page { display: flex; flex-grow: 1; min-width: 0; min-height: 0; }
  .tree { flex: 0 1 var(--w, 280px); min-width: 200px; display: flex; flex-direction: column; border-right: 1px solid var(--line); background: var(--surface-alt); }
  .head { padding: 14px 16px 8px; display: flex; flex-direction: column; gap: 8px; }
  .filter-box input { width: 100%; height: 32px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 13px; color: var(--ink); outline: none; }
  .filter-box input:focus { border-color: var(--accent); }
  .hide-empty { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--muted); }
  .hide-empty input { accent-color: var(--accent); }
  .parent-row { display: flex; align-items: center; border-radius: 6px; }
  .parent-row .row { flex-grow: 1; min-width: 0; padding-left: 2px; }
  .exp { width: 22px; height: 30px; border: none; background: none; color: var(--muted); flex-shrink: 0; padding: 0; }
  .chev { display: inline-block; transition: transform .12s; font-size: 16px; }
  .chev.open { transform: rotate(90deg); }
  .row.zero { opacity: .55; }
  h2 { margin: 0; font-size: 16px; font-weight: 600; }
  .scroll { flex-grow: 1; overflow-y: auto; padding: 0 8px 8px; }
  .row { display: flex; justify-content: space-between; align-items: center; height: 34px; padding: 0 10px; border-radius: 6px; font-size: 14px; color: var(--ink); text-decoration: none; }
  .row.child { padding-left: 32px; font-size: 13px; color: var(--muted-2); }
  .row:hover { background: var(--surface-hover); text-decoration: none; }
  .row.active { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 500; }
  .n { color: var(--muted); font-size: 12px; }
  .placeholder { flex-grow: 1; display: flex; align-items: center; justify-content: center; color: var(--muted); background: var(--surface); }
  @media (max-width: 900px) {
    .genres-page > :global(*) { display: none; }
    .genres-page.show-list > .tree { display: flex; width: 100%; flex: 1 1 auto; }
    .genres-page.show-books > :global(main) { display: flex; width: 100%; }
  }
</style>
