<script lang="ts">
  import NameBrowser from '../components/NameBrowser.svelte';
  import BooksPane from '../components/BooksPane.svelte';
  import DetailsPane from '../components/DetailsPane.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import ShelfDialog from '../components/ShelfDialog.svelte';
  import { loadNameList } from '../cache/nameCache';
  import { currentLibrary } from '../stores/libraries.svelte';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';
  import type { NameListResponse } from '../api/types';

  let { kind, lib, id }: { kind: 'authors' | 'series'; lib: number; id: number | null } = $props();

  let list = $state<NameListResponse | null>(null);
  let selectedBookId = $state<number | null>(null);
  let sendIds = $state<number[] | null>(null);
  let shelfIds = $state<number[] | null>(null);
  let mobilePane = $state<'list' | 'books' | 'detail'>('list');

  $effect(() => {
    const libObj = currentLibrary();
    if (!libObj) return;
    loadNameList(lib, kind, libObj.catalogVersion).then((res) => { list = res; });
  });

  $effect(() => {
    selectedBookId = null;
    mobilePane = id ? 'books' : 'list';
  });

  function selectName(newId: number) {
    navigate(`/l/${lib}/${kind}/${newId}`);
  }

  function pickBook(bid: number) {
    selectedBookId = bid;
    if (window.innerWidth < 900) navigate(`/l/${lib}/book/${bid}`);
  }

  const rows = $derived(list?.rows ?? []);
  const letters = $derived(list?.letters ?? []);
  const current = $derived(rows.find((r) => r[0] === id) ?? null);
  const title = $derived(kind === 'authors' ? t('nav.authors') : t('nav.series'));
  const filterLabel = $derived(kind === 'authors' ? t('authors.filterLabel') : t('series.filterLabel'));

  let liveBooksCount = $state(0);
  let liveSeriesCount = $state(0);
  let liveCoauthors = $state<{ id: number; name: string }[]>([]);
  $effect(() => {
    liveBooksCount = current ? current[2] : 0;
    liveSeriesCount = 0;
    liveCoauthors = [];
  });

  const crumbLetter = $derived(current ? current[1].charAt(0).toUpperCase() : '');
  // "also with" (prototype: Main.dc.html): authors this one co-wrote books
  // with, i.e. who appear alongside them on at least one loaded book.
  const alsoWith = $derived(
    kind === 'authors' && liveCoauthors.length
      ? liveCoauthors.map((a) => ({ id: a.id, name: a.name, href: `/l/${lib}/authors/${a.id}` }))
      : undefined,
  );
  const header = $derived(
    current
      ? {
          crumb: `${title} / ${crumbLetter}`,
          name: current[1],
          booksCount: liveBooksCount,
          seriesCount: kind === 'authors' ? liveSeriesCount : undefined,
          alsoWith,
        }
      : undefined,
  );
</script>

<div class="browse" class:show-list={mobilePane === 'list'} class:show-books={mobilePane === 'books'}>
  <NameBrowser {title} {filterLabel} {rows} {letters} selectedId={id} onSelect={selectName} />

  {#if id !== null && current}
    <BooksPane
      {lib}
      scope={kind === 'authors' ? { kind: 'author', id, groupable: true } : { kind: 'series', id, groupable: false }}
      {selectedBookId}
      onPick={pickBook}
      onOpenSend={(ids) => (sendIds = ids)}
      onOpenShelf={(ids) => (shelfIds = ids)}
      {header}
      onBack={() => (mobilePane = 'list')}
      onCounts={(c) => { liveBooksCount = c.books; liveSeriesCount = c.series; liveCoauthors = c.coauthors; }}
    />
    <DetailsPane
      {lib}
      bookId={selectedBookId}
      onSend={(ids) => (sendIds = ids)}
      onAddShelf={(ids) => (shelfIds = ids)}
    />
  {:else}
    <div class="placeholder">{t('details.noSelection')}</div>
  {/if}
</div>

{#if sendIds}
  <SendDialog {lib} bookIds={sendIds} open={true} onClose={() => (sendIds = null)} />
{/if}
{#if shelfIds}
  <ShelfDialog {lib} bookIds={shelfIds} open={true} onClose={() => (shelfIds = null)} />
{/if}

<style>
  .browse { display: flex; flex-grow: 1; min-width: 0; min-height: 0; }
  .placeholder { flex-grow: 1; display: flex; align-items: center; justify-content: center; color: var(--muted); background: var(--surface); }

  @media (max-width: 900px) {
    .browse > :global(*) { display: none; }
    .browse.show-list > :global(section) { display: flex; width: 100%; }
    .browse.show-books > :global(main) { display: flex; width: 100%; }
    .browse:not(.show-list):not(.show-books) > :global(aside) { display: flex; width: 100%; }
  }
</style>
