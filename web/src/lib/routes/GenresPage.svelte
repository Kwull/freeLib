<script lang="ts">
  import { api } from '../api/client';
  import type { Genre } from '../api/types';
  import BooksPane from '../components/BooksPane.svelte';
  import DetailsPane from '../components/DetailsPane.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import ShelfDialog from '../components/ShelfDialog.svelte';
  import { navigate } from '../router.svelte';
  import { t, i18nState } from '../i18n';

  let { lib, id }: { lib: number; id: number | null } = $props();

  let genres = $state<Genre[]>([]);
  let selectedBookId = $state<number | null>(null);
  let sendIds = $state<number[] | null>(null);
  let shelfIds = $state<number[] | null>(null);

  $effect(() => { const lang = i18nState.lang; api.genres(lib, lang).then((g) => (genres = g)); });
  $effect(() => { selectedBookId = null; });

  const top = $derived(genres.filter((g) => g.parent === 0));
  const childrenOf = (pid: number) => genres.filter((g) => g.parent === pid);
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
  <section aria-label={t('genres.title')} class="tree">
    <div class="head"><h2>{t('genres.title')}</h2></div>
    <div class="scroll">
      {#each top as g (g.id)}
        <a href="/l/{lib}/genres/{g.id}" data-link class="row parent" class:active={g.id === id}>{g.name}<span class="n">{g.count}</span></a>
        {#each childrenOf(g.id) as c (c.id)}
          <a href="/l/{lib}/genres/{c.id}" data-link class="row child" class:active={c.id === id}>{c.name}<span class="n">{c.count}</span></a>
        {/each}
      {/each}
    </div>
  </section>

  {#if id !== null && current}
    <BooksPane
      {lib}
      scope={{ kind: 'genre', id, groupable: false }}
      {selectedBookId}
      onPick={pickBook}
      onOpenSend={(ids) => (sendIds = ids)}
      onOpenShelf={(ids) => (shelfIds = ids)}
      {header}
      onBack={() => (mobilePane = 'list')}
      onCounts={(c) => (liveBooksCount = c.books)}
    />
    <DetailsPane {lib} bookId={selectedBookId} onSend={(ids) => (sendIds = ids)} onAddShelf={(ids) => (shelfIds = ids)} />
  {:else}
    <div class="placeholder">{t('details.noSelection')}</div>
  {/if}
</div>

{#if sendIds}<SendDialog {lib} bookIds={sendIds} open={true} onClose={() => (sendIds = null)} />{/if}
{#if shelfIds}<ShelfDialog {lib} bookIds={shelfIds} open={true} onClose={() => (shelfIds = null)} />{/if}

<style>
  .genres-page { display: flex; flex-grow: 1; min-width: 0; min-height: 0; }
  .tree { width: 280px; flex-shrink: 0; display: flex; flex-direction: column; border-right: 1px solid var(--line); background: var(--surface-alt); }
  .head { padding: 16px; }
  h2 { margin: 0; font-size: 16px; font-weight: 600; }
  .scroll { flex-grow: 1; overflow-y: auto; padding: 0 8px 8px; }
  .row { display: flex; justify-content: space-between; align-items: center; height: 34px; padding: 0 10px; border-radius: 6px; font-size: 14px; color: var(--ink); text-decoration: none; }
  .row.child { padding-left: 24px; font-size: 13px; color: var(--muted-2); }
  .row:hover { background: var(--surface-hover); text-decoration: none; }
  .row.active { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 500; }
  .n { color: var(--muted); font-size: 12px; }
  .placeholder { flex-grow: 1; display: flex; align-items: center; justify-content: center; color: var(--muted); background: var(--surface); }
  @media (max-width: 900px) {
    .genres-page > :global(*) { display: none; }
    .genres-page.show-list > .tree { display: flex; width: 100%; }
    .genres-page.show-books > :global(main) { display: flex; width: 100%; }
  }
</style>
