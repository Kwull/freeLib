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
  import { api, errorText } from '../api/client';
  import StateCard from '../components/StateCard.svelte';
  import Spinner from '../components/Spinner.svelte';
  import Icon from '../components/Icon.svelte';
  import { t, tn, i18nState } from '../i18n';
  import { normalize, letterOf } from '../utils/normalize';
  import type { AuthorSummary, NameListResponse } from '../api/types';

  let { kind, lib, id }: { kind: 'authors' | 'series'; lib: number; id: number | null } = $props();

  let list = $state<NameListResponse | null>(null);
  let listError = $state<string | null>(null);
  let listProgress = $state<{ bytes: number; rows: number } | null>(null);
  let reloadTick = $state(0);
  let selectedBookId = $state<number | null>(null);
  let send = $state<{ ids: number[]; device?: number; series?: number[] } | null>(null);
  let shelfIds = $state<number[] | null>(null);
  let mobilePane = $state<'list' | 'books' | 'detail'>('list');
  let summary = $state<AuthorSummary | null>(null);
  let liveListWidth = $state<number | null>(null);

  // only a new catalog version reloads the list (not every library status update)
  const catalogVersion = $derived(currentLibrary()?.catalogVersion ?? null);
  $effect(() => {
    const version = catalogVersion;
    reloadTick;
    if (version === null) return;
    let cancelled = false;
    listError = null;
    listProgress = null;
    list = null;
    loadNameList(lib, kind, version, (p) => { if (!cancelled) listProgress = p; })
      .then((res) => { if (!cancelled) list = res; })
      .catch((e) => { if (!cancelled) listError = errorText(e); });
    return () => { cancelled = true; };
  });

  /** "12,345 of 200,000 authors · 4.2 MB" while a big list downloads. */
  const progressText = $derived.by(() => {
    if (list || !listProgress) return null;
    const libObj = currentLibrary();
    const expected = libObj ? (kind === 'authors' ? libObj.authorCount : libObj.seriesCount) : 0;
    const fmt = new Intl.NumberFormat(i18nState.lang);
    const mb = (listProgress.bytes / 1048576).toFixed(1);
    return expected > 0
      ? t('browse.loadingRows', { rows: fmt.format(Math.min(listProgress.rows, expected)), total: fmt.format(expected), mb })
      : t('browse.loadingBytes', { mb });
  });
  const progressValue = $derived.by(() => {
    const libObj = currentLibrary();
    const expected = libObj ? (kind === 'authors' ? libObj.authorCount : libObj.seriesCount) : 0;
    if (!listProgress || !expected) return null;
    return Math.min(0.99, listProgress.rows / expected);
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
  <NameBrowser {title} {filterLabel} {rows} {letters} selectedId={id} onSelect={selectName} width={listWidth}
    loading={!list && !listError} {progressText} progress={progressValue} />
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
      onSendSeries={(series) => (send = { ids: [], series })}
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
  {:else if listError}
    <StateCard tone="error" icon="alert" title={kind === 'authors' ? t('browse.authorsError') : t('browse.seriesError')} detail={listError} testid="list-error">
      {#snippet actions()}
        <button type="button" class="primary" onclick={() => reloadTick++}><Icon name="refresh" size={16} />{t('common.retry')}</button>
      {/snippet}
    </StateCard>
  {:else}
    <div class="placeholder">
      {#if id !== null && list}
        <p>{kind === 'authors' ? t('browse.noSuchAuthor') : t('browse.noSuchSeries')}</p>
      {:else if list}
        <p class="big">{kind === 'authors' ? tn('browse.authorsTotal', rows.length) : tn('browse.seriesTotal', rows.length)}</p>
        <p>{kind === 'authors' ? t('browse.pickAuthor') : t('browse.pickSeries')}</p>
      {:else}
        <Spinner size={32} />
        <p class="loading-label">{kind === 'authors' ? t('browse.loadingAuthors') : t('browse.loadingSeries')}</p>
        {#if progressText}<p class="loading-detail" data-testid="list-progress">{progressText}</p>{/if}
      {/if}
    </div>
  {/if}
</div>

{#if send}
  <SendDialog {lib} bookIds={send.ids} device={send.device} seriesIds={send.series} open={true} onClose={() => (send = null)} />
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
  .placeholder .loading-label { margin-top: 8px; color: var(--muted-2); font-size: 15px; }
  .placeholder .loading-detail { font-size: 13px; font-variant-numeric: tabular-nums; }
  .placeholder .big { font-family: var(--font-display); font-size: 22px; color: var(--ink); }

  @media (max-width: 900px) {
    .browse > :global(*) { display: none; }
    .browse.show-list > :global(section) { display: flex; width: 100%; }
    .browse.show-books > :global(main) { display: flex; width: 100%; }
    .browse:not(.show-list):not(.show-books) > :global(aside) { display: flex; width: 100%; }
  }
</style>
