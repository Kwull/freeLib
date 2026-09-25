<script lang="ts">
  import Shell from './lib/components/Shell.svelte';
  import LoginPage from './lib/routes/LoginPage.svelte';
  import BrowsePage from './lib/routes/BrowsePage.svelte';
  import GenresPage from './lib/routes/GenresPage.svelte';
  import NewArrivalsPage from './lib/routes/NewArrivalsPage.svelte';
  import ShelfPage from './lib/routes/ShelfPage.svelte';
  import ShelvesIndexPage from './lib/routes/ShelvesIndexPage.svelte';
  import SearchPage from './lib/routes/SearchPage.svelte';
  import LibrariesPage from './lib/routes/LibrariesPage.svelte';
  import BookPhonePage from './lib/routes/BookPhonePage.svelte';

  import { currentRoute, navigate } from './lib/router.svelte';
  import { loadSession, sessionState, isLoggedIn } from './lib/stores/session.svelte';
  import { loadLibraries, librariesState, currentLibrary, setCurrentLibrary } from './lib/stores/libraries.svelte';
  import { loadDevices } from './lib/stores/devices.svelte';
  import { loadShelves } from './lib/stores/shelves.svelte';
  import { startJobEvents, loadJobs } from './lib/stores/jobs.svelte';
  import { applyTheme } from './lib/stores/theme.svelte';
  import { i18nState } from './lib/i18n';
  import { toastState } from './lib/stores/toast.svelte';
  import { loadPrefs, getPref, setPref } from './lib/stores/prefs.svelte';

  let ready = $state(false);
  let readerComp = $state<any>(null);
  let settingsComp = $state<any>(null);

  applyTheme();
  document.documentElement.lang = i18nState.lang;

  $effect(() => {
    (async () => {
      await loadSession();
      if (isLoggedIn()) {
        await Promise.all([loadLibraries(), loadDevices(), loadShelves(), loadJobs(), loadPrefs()]);
        startJobEvents();
        const preferred = getPref<number | null>('currentLibrary', null);
        if (typeof preferred === 'number' && librariesState.items.some((l) => l.id === preferred)) setCurrentLibrary(preferred);
      }
      ready = true;
    })();
  });

  $effect(() => {
    const id = librariesState.currentId;
    if (ready && id !== null && getPref('currentLibrary', null) !== id) setPref('currentLibrary', id);
  });

  $effect(() => {
    const route = currentRoute();
    if (route.name === 'read') import('./lib/routes/ReaderPage.svelte').then((m) => (readerComp = m.default));
    if (route.name === 'settings') import('./lib/routes/SettingsPage.svelte').then((m) => (settingsComp = m.default));
  });

  // Redirect bare "/" to the default library's authors page once libraries are known.
  $effect(() => {
    if (!ready || !isLoggedIn()) return;
    const route = currentRoute();
    if (route.name === 'home') {
      const lib = currentLibrary();
      if (lib) navigate(`/l/${lib.id}/authors`, { replace: true });
    }
  });
</script>

{#if !ready}
  <div class="boot">Loading…</div>
{:else if !isLoggedIn()}
  <LoginPage />
{:else}
  {@const route = currentRoute()}
  <Shell>
    {#if route.name === 'home'}
      <div class="boot">Loading…</div>
    {:else if route.name === 'authors' || route.name === 'series'}
      <BrowsePage kind={route.name} lib={route.lib} id={route.id} />
    {:else if route.name === 'genres'}
      <GenresPage lib={route.lib} id={route.id} />
    {:else if route.name === 'new'}
      <NewArrivalsPage lib={route.lib} />
    {:else if route.name === 'shelvesIndex'}
      <ShelvesIndexPage lib={route.lib} />
    {:else if route.name === 'shelf'}
      <ShelfPage lib={route.lib} id={route.id} />
    {:else if route.name === 'search'}
      <SearchPage lib={route.lib} q={route.q} />
    {:else if route.name === 'book'}
      <BookPhonePage lib={route.lib} id={route.id} />
    {:else if route.name === 'read'}
      {#if readerComp}
        {@const Comp = readerComp}
        <Comp lib={route.lib} id={route.id} />
      {/if}
    {:else if route.name === 'libraries'}
      <LibrariesPage />
    {:else if route.name === 'settings'}
      {#if settingsComp}
        {@const Comp = settingsComp}
        <Comp section={route.section} />
      {/if}
    {:else if route.name === 'login'}
      <LoginPage />
    {:else}
      <div class="not-found">404</div>
    {/if}
  </Shell>
{/if}

<div class="toasts" aria-live="polite">
  {#each toastState.items as tst (tst.id)}
    <div class="toast" class:error={tst.kind === 'error'}>{tst.text}</div>
  {/each}
</div>

<style>
  .boot, .not-found { height: 100%; display: flex; align-items: center; justify-content: center; color: var(--muted); }
  .toasts { position: fixed; bottom: 20px; right: 20px; display: flex; flex-direction: column; gap: 8px; z-index: 200; }
  .toast { padding: 10px 16px; border-radius: 8px; background: var(--inverse-bg); color: var(--inverse-ink); font-size: 14px; box-shadow: 0 8px 24px rgba(0,0,0,.2); max-width: 420px; }
  .toast.error { background: var(--danger); color: #fff; }
  @media (max-width: 900px) {
    .toasts { bottom: 84px; left: 12px; right: 12px; }
  }
</style>
