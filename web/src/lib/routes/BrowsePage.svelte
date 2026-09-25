<script lang="ts">
  import NameBrowser from '../components/NameBrowser.svelte';
  import BooksPane from '../components/BooksPane.svelte';
  import DetailsPane from '../components/DetailsPane.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import ShelfDialog from '../components/ShelfDialog.svelte';
  import Splitter from '../components/Splitter.svelte';
  import { loadNameList } from '../cache/nameCache';
  import { currentLibrary } from '../stores/libraries.svelte';
  import { PANE_LIMITS, paneWidth, setPaneWidth } from '../stores/layout.svelte';
  import { navigate, routerState } from '../router.svelte';
  import { api } from '../api/client';
  import { t, tn } from '../i18n';
  import { normalize, letterOf } from '../utils/normalize';
  import type { AuthorSummary, NameListResponse } from '../api/types';

  let { kind, lib, id }: { kind: 'authors' | 'series'; lib: number; id: number | null } = $props();

  let list = $state<NameListResponse | null>(null);
  let listError = $state(false);
  let selectedBookId = $state<number | null>(null);
  let send = $state<{ ids: number[]; device?: number } | null>(null);
  let shelfIds = $state<number[] | null>(null);
  let mobilePane = $state<'list' | 'books' | 'detail'>('list');
  let summary = $state<AuthorSummary | null>(null);
  let liveListWidth = $state<number | null>(null);

  $effect(() => {
    const libObj = currentLibrary();
    if (!libObj) return;
    listError = false;
    loadNameList(lib, kind, libObj.catalogVersion).then((res) => { list = res; }).catch(() => { listError = true; });
  });

  // `?book=<id>` (from the search box) preselects a book of this author/series
  const bookParam = $derived(Number(new URLSearchParams(routerState.search).get('book')) || null);
  $effect(() => {
    id;
    selectedBookId = bookParam;
    mobilePane = id ? 'books' : 'list';
  });

  // Author overview: header counts, top co-authors, the details pane's summary.
  $effect(() => {
    summary = null;
    if (kind !== 'authors' || id === null) return;
    const aid = id;
    let cancelled = false;
    api.authorSummary(lib, aid).then((s) => { if (!cancelled) summary = s; }).catch(() => {});
    return () => { cancelled = true; };
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
  const listWidth = $derived(liveListWidth ?? paneWidth('list'));

  const header = $derived(
    current
      ? {
          crumb: `${title} / ${letterOf(normalize(current[1]))}`,
          name: current[1],
          booksCount: summary?.id === id ? summary.count : current[2],
          seriesCount: kind === 'authors' && summary?.id === id ? summary.series.length : undefined,
          anthologies: kind === 'authors' && summary?.id === id ? summary.anthologies : undefined,
          coauthors: kind === 'authors' && summary?.id === id ? summary.coauthors : undefined,
          coauthorCount: kind === 'authors' && summary?.id === id ? summary.coauthorCount : undefined,
        }
      : undefined,
  );
</script>

<div class="browse" class:show-list={mobilePane === 'list'} class:show-books={mobilePane === 'books'}>
  <NameBrowser {title} {filterLabel} {rows} {letters} selectedId={id} onSelect={selectName} width={listWidth} />
  <Splitter
    value={listWidth}
    min={PANE_LIMITS.list.min}
    max={PANE_LIMITS.list.max}
    label={t('layout.resizeList')}
    onInput={(v) => (liveListWidth = v)}
    onCommit={(v) => { setPaneWidth('list', v); liveListWidth = null; }}
    onReset={() => { setPaneWidth('list', null); liveListWidth = null; }}
  />

  {#if id !== null && current}
    <BooksPane
      {lib}
      scope={kind === 'authors' ? { kind: 'author', id, groupable: true } : { kind: 'series', id, groupable: false }}
      {selectedBookId}
      onPick={pickBook}
      onOpenSend={(ids) => (send = { ids })}
      onOpenShelf={(ids) => (shelfIds = ids)}
      {header}
      onBack={() => (mobilePane = 'list')}
    />
    <DetailsPane
      {lib}
      bookId={selectedBookId}
      summary={kind === 'authors' && summary?.id === id ? summary : null}
      onSend={(ids, device) => (send = { ids, device })}
      onAddShelf={(ids) => (shelfIds = ids)}
    />
  {:else}
    <div class="placeholder">
      {#if listError}
        <p>{t('common.error')}</p>
      {:else if id !== null && list}
        <p>{kind === 'authors' ? t('browse.noSuchAuthor') : t('browse.noSuchSeries')}</p>
      {:else if list}
        <p class="big">{kind === 'authors' ? tn('browse.authorsTotal', rows.length) : tn('browse.seriesTotal', rows.length)}</p>
        <p>{kind === 'authors' ? t('browse.pickAuthor') : t('browse.pickSeries')}</p>
      {:else}
        <p>{t('common.loading')}</p>
      {/if}
    </div>
  {/if}
</div>

{#if send}
  <SendDialog {lib} bookIds={send.ids} device={send.device} open={true} onClose={() => (send = null)} />
{/if}
{#if shelfIds}
  <ShelfDialog {lib} bookIds={shelfIds} open={true} onClose={() => (shelfIds = null)} />
{/if}

<style>
  .browse { display: flex; flex-grow: 1; min-width: 0; min-height: 0; }
  .placeholder {
    flex-grow: 1; display: flex; flex-direction: column; gap: 6px; align-items: center; justify-content: center;
    color: var(--muted); background: var(--surface); padding: 24px; text-align: center;
  }
  .placeholder p { margin: 0; max-width: 420px; }
  .placeholder .big { font-family: var(--font-display); font-size: 22px; color: var(--ink); }

  @media (max-width: 900px) {
    .browse > :global(*) { display: none; }
    .browse.show-list > :global(section) { display: flex; width: 100%; }
    .browse.show-books > :global(main) { display: flex; width: 100%; }
    .browse:not(.show-list):not(.show-books) > :global(aside) { display: flex; width: 100%; }
  }
</style>
