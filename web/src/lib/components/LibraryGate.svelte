<script lang="ts">
  // Renders a library page only when there is a catalog to browse; otherwise one of the
  // library states: loading (skeleton panes), the library list failed (retry), no libraries
  // yet (first run), unknown library, importing / rebuilding after an update (live progress
  // from SSE, the page continues by itself when done), import failed, never imported.
  import { untrack, type Snippet } from 'svelte';
  import StateCard from './StateCard.svelte';
  import BrowseSkeleton from './BrowseSkeleton.svelte';
  import Icon from './Icon.svelte';
  import { t } from '../i18n';
  import { api, errorText } from '../api/client';
  import {
    librariesState, loadLibraries, currentLibrary, setCurrentLibrary, isBrowsable, upsertLibrary,
  } from '../stores/libraries.svelte';
  import { jobsState, toggleActivity } from '../stores/jobs.svelte';
  import { sessionState } from '../stores/session.svelte';
  import { showToast } from '../stores/toast.svelte';
  import { navigate } from '../router.svelte';
  import type { Library } from '../api/types';

  let { lib, children }: { lib: number | null; children: Snippet } = $props();

  const isAdmin = $derived(sessionState.openMode || sessionState.user?.role === 'admin');
  const L = $derived<Library | null>(
    lib === null ? currentLibrary() : librariesState.items.find((l) => l.id === lib) ?? null,
  );
  const others = $derived(librariesState.items.filter((l) => l.id !== L?.id && isBrowsable(l)));
  const fallback = $derived(
    librariesState.items.find((l) => l.isDefault && isBrowsable(l)) ?? librariesState.items.find(isBrowsable) ?? librariesState.items[0] ?? null,
  );

  // The library in the URL is the current one (switcher, navigation, search).
  $effect(() => {
    if (lib !== null && librariesState.items.some((l) => l.id === lib) && librariesState.currentId !== lib) {
      setCurrentLibrary(lib);
    }
  });

  // Without the event stream (a proxy that buffers SSE, a dropped connection) poll instead.
  $effect(() => {
    if (!L || L.status.state !== 'importing' || jobsState.eventsConnected) return;
    const timer = setInterval(() => { loadLibraries().catch(() => {}); }, 4000);
    return () => clearInterval(timer);
  });

  // Say so when the library this page waited for becomes browsable.
  let waitingFor: number | null = null;
  $effect(() => {
    if (!L) return;
    if (!isBrowsable(L)) waitingFor = L.id;
    else if (waitingFor === L.id) {
      waitingFor = null;
      const name = L.name;
      untrack(() => showToast(t('libstate.ready', { name })));
    }
  });

  let busy = $state(false);
  async function retryList() {
    busy = true;
    try { await loadLibraries(); } catch { /* the card shows the error */ } finally { busy = false; }
  }
  async function startImport(l: Library) {
    busy = true;
    try {
      await api.importLibrary(l.id, 'full');
      upsertLibrary({ ...l, status: { state: 'importing', progress: 0 } });
      loadLibraries().catch(() => {});
    } catch (e) {
      showToast(errorText(e), 'error');
    } finally {
      busy = false;
    }
  }
  function openLibrary(l: Library) {
    setCurrentLibrary(l.id);
    navigate(`/l/${l.id}/authors`);
  }
</script>

{#if !librariesState.loaded}
  {#if librariesState.error}
    <StateCard tone="error" icon="alert" title={t('libstate.listError')} text={t('libstate.listErrorText')} detail={librariesState.error} testid="libraries-error">
      {#snippet actions()}
        <button type="button" class="primary" disabled={busy} onclick={retryList}><Icon name="refresh" size={16} />{t('common.retry')}</button>
      {/snippet}
    </StateCard>
  {:else}
    <BrowseSkeleton label={t('libstate.loading')} />
  {/if}
{:else if librariesState.items.length === 0}
  {#if isAdmin}
    <StateCard icon="library" title={t('libstate.firstRun')} text={t('libstate.firstRunAdmin')} testid="first-run">
      {#snippet actions()}
        <a class="primary" href="/libraries?add=1" data-link><Icon name="plus" size={16} />{t('libstate.addFirst')}</a>
      {/snippet}
    </StateCard>
  {:else}
    <StateCard icon="library" title={t('libstate.noLibraries')} text={t('libstate.noLibrariesReader')} testid="first-run" />
  {/if}
{:else if lib !== null && !L}
  <StateCard icon="alert" title={t('libstate.notFound')} text={t('libstate.notFoundText')} testid="library-not-found">
    {#snippet actions()}
      {#if fallback}<button type="button" class="primary" onclick={() => openLibrary(fallback)}>{t('libstate.openLibrary', { name: fallback.name })}</button>{/if}
    {/snippet}
  </StateCard>
{:else if L && !isBrowsable(L)}
  {#if L.status.state === 'importing'}
    {@const rebuild = L.status.reason === 'upgrade'}
    <StateCard
      busy
      title={rebuild ? t('libstate.rebuilding') : t('libstate.importing')}
      text={rebuild ? t('libstate.rebuildingText', { name: L.name }) : t('libstate.importingText', { name: L.name })}
      progress={L.status.progress ?? null}
      step={L.status.message ?? t('libstate.starting')}
      testid="library-importing"
    >
      {#if others.length}
        <div class="others">
          <p>{t('libstate.meanwhile')}</p>
          <div class="other-list">
            {#each others as o (o.id)}
              <button type="button" onclick={() => openLibrary(o)}><Icon name="library" size={16} />{o.name}</button>
            {/each}
          </div>
        </div>
      {/if}
      {#snippet actions()}
        <button type="button" onclick={() => toggleActivity(true)}><Icon name="activity" size={16} />{t('libstate.showActivity')}</button>
      {/snippet}
    </StateCard>
  {:else if L.status.state === 'error'}
    <StateCard tone="error" icon="alert" title={t('libstate.failed')} text={isAdmin ? t('libstate.failedAdmin') : t('libstate.failedReader')} detail={L.status.message} testid="library-error">
      {#snippet actions()}
        {#if isAdmin && L.inpx}<button type="button" class="primary" disabled={busy} onclick={() => startImport(L)}><Icon name="refresh" size={16} />{t('libstate.retryImport')}</button>{/if}
        {#if isAdmin}<a href="/libraries" data-link>{t('libstate.librarySettings')}</a>{/if}
        {#each others.slice(0, 2) as o (o.id)}<button type="button" onclick={() => openLibrary(o)}>{t('libstate.openLibrary', { name: o.name })}</button>{/each}
      {/snippet}
    </StateCard>
  {:else}
    <StateCard icon="library" title={t('libstate.notImported')} text={isAdmin ? (L.inpx ? t('libstate.notImportedAdmin') : t('libstate.noInpx')) : t('libstate.notImportedReader')} testid="library-not-imported">
      {#snippet actions()}
        {#if isAdmin && L.inpx}<button type="button" class="primary" disabled={busy} onclick={() => startImport(L)}>{t('libstate.importNow')}</button>{/if}
        {#if isAdmin}<a href="/libraries" data-link>{t('libstate.librarySettings')}</a>{/if}
      {/snippet}
    </StateCard>
  {/if}
{:else if !L}
  <BrowseSkeleton label={t('libstate.loading')} />
{:else}
  {@render children()}
{/if}

<style>
  .others { width: 100%; margin-top: 14px; padding-top: 14px; border-top: 1px solid var(--line-soft); }
  .others p { margin: 0 0 8px; font-size: 13px; color: var(--muted); }
  .other-list { display: flex; flex-wrap: wrap; justify-content: center; gap: 6px; }
  .other-list button {
    display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 12px; border-radius: 16px;
    border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 13px;
  }
  .other-list button:hover { background: var(--surface-hover); }
</style>
