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
  import OAuthConsentPage from './lib/routes/OAuthConsentPage.svelte';
  import HomePage from './lib/routes/HomePage.svelte';

  import { currentRoute, navigate } from './lib/router.svelte';
  import { loadSession, isLoggedIn } from './lib/stores/session.svelte';
  import { errorText } from './lib/api/client';
  import ShellSkeleton from './lib/components/ShellSkeleton.svelte';
  import StateCard from './lib/components/StateCard.svelte';
  import LibraryGate from './lib/components/LibraryGate.svelte';
  import BrowseSkeleton from './lib/components/BrowseSkeleton.svelte';
  import Icon from './lib/components/Icon.svelte';
  import { t } from './lib/i18n';
  import { loadLibraries, librariesState, currentLibrary, setCurrentLibrary } from './lib/stores/libraries.svelte';
  import { loadDevices } from './lib/stores/devices.svelte';
  import { loadShelves } from './lib/stores/shelves.svelte';
  import { startJobEvents, loadJobs } from './lib/stores/jobs.svelte';
  import { applyTheme } from './lib/stores/theme.svelte';
  import { i18nState } from './lib/i18n';
  import { toastState } from './lib/stores/toast.svelte';
  import { loadPrefs, getPref, setPref } from './lib/stores/prefs.svelte';

  // boot: the session is loading (skeleton frame), failed (card + retry), or known
  let boot = $state<'loading' | 'error' | 'ready'>('loading');
  let bootError = $state('');
  let bootSlow = $state(false);
  // libraries + prefs are loaded and the preferred library is applied
  let libsReady = $state(false);
  let started = false;
  let readerComp = $state<any>(null);
  let settingsComp = $state<any>(null);

  applyTheme();
  document.documentElement.lang = i18nState.lang;

  async function start() {
    boot = 'loading';
    bootSlow = false;
    const slow = setTimeout(() => (bootSlow = true), 6000);
    try {
      await loadSession();
      boot = 'ready';
    } catch (e) {
      bootError = errorText(e);
      boot = 'error';
    } finally {
      clearTimeout(slow);
    }
  }
  start();

  // After the session (and after a login): the shell renders at once; its data loads behind it.
  $effect(() => {
    if (boot !== 'ready') return;
    if (!isLoggedIn()) { started = false; libsReady = false; return; }
    if (started) return;
    started = true;
    loadDevices().catch(() => {});
    loadShelves().catch(() => {});
    loadJobs().catch(() => {});
    startJobEvents();
    Promise.all([loadPrefs(), loadLibraries()])
      .then(() => {
        const preferred = getPref<number | null>('currentLibrary', null);
        if (typeof preferred === 'number' && librariesState.items.some((l) => l.id === preferred)) setCurrentLibrary(preferred);
      })
      .catch(() => { /* LibraryGate shows the error and retries */ })
      .finally(() => (libsReady = true));
  });

  $effect(() => {
    const id = librariesState.currentId;
    if (libsReady && id !== null && getPref('currentLibrary', null) !== id) setPref('currentLibrary', id);
  });

  $effect(() => {
    const route = currentRoute();
    if (route.name === 'read') import('./lib/routes/ReaderPage.svelte').then((m) => (readerComp = m.default));
    if (route.name === 'settings') import('./lib/routes/SettingsPage.svelte').then((m) => (settingsComp = m.default));
  });

  // Redirect bare "/" to the current library's start page once libraries are known.
  $effect(() => {
    if (!libsReady || !isLoggedIn()) return;
    const route = currentRoute();
    if (route.name === 'home' || route.name === 'login') {
      const lib = currentLibrary();
      if (lib) navigate(`/l/${lib.id}/home`, { replace: true });
    }
  });
</script>

{#if boot === 'loading'}
  <ShellSkeleton label={bootSlow ? t('boot.slow') : t('boot.loading')} />
{:else if boot === 'error'}
  <div class="boot-error">
    <StateCard tone="error" icon="alert" title={t('boot.error')} text={t('boot.errorText')} detail={bootError} testid="boot-error">
      {#snippet actions()}
        <button type="button" class="primary" onclick={start}><Icon name="refresh" size={16} />{t('common.retry')}</button>
      {/snippet}
    </StateCard>
  </div>
{:else if currentRoute().name === 'oauthConsent' && (isLoggedIn() || (currentRoute() as { error: string | null }).error)}
  {@const r = currentRoute() as { request: string | null; error: string | null }}
  <OAuthConsentPage request={r.request} error={r.error} />
{:else if !isLoggedIn()}
  <LoginPage />
{:else}
  {@const route = currentRoute()}
  <Shell>
    {#if route.name === 'home'}
      <LibraryGate lib={null}><BrowseSkeleton label={t('libstate.loading')} /></LibraryGate>
    {:else if route.name === 'start'}
      <LibraryGate lib={route.lib}><HomePage lib={route.lib} /></LibraryGate>
    {:else if route.name === 'authors' || route.name === 'series'}
      <LibraryGate lib={route.lib}><BrowsePage kind={route.name} lib={route.lib} id={route.id} /></LibraryGate>
    {:else if route.name === 'genres'}
      <LibraryGate lib={route.lib}><GenresPage lib={route.lib} id={route.id} /></LibraryGate>
    {:else if route.name === 'new'}
      <LibraryGate lib={route.lib}><NewArrivalsPage lib={route.lib} /></LibraryGate>
    {:else if route.name === 'shelvesIndex'}
      <LibraryGate lib={route.lib}><ShelvesIndexPage lib={route.lib} /></LibraryGate>
    {:else if route.name === 'shelf'}
      <LibraryGate lib={route.lib}><ShelfPage lib={route.lib} id={route.id} /></LibraryGate>
    {:else if route.name === 'search'}
      <LibraryGate lib={route.lib}><SearchPage lib={route.lib} q={route.q} /></LibraryGate>
    {:else if route.name === 'book'}
      <LibraryGate lib={route.lib}><BookPhonePage lib={route.lib} id={route.id} /></LibraryGate>
    {:else if route.name === 'read'}
      <LibraryGate lib={route.lib}>
        {#if readerComp}
          {@const Comp = readerComp}
          <Comp lib={route.lib} id={route.id} />
        {/if}
      </LibraryGate>
    {:else if route.name === 'libraries'}
      <LibrariesPage />
    {:else if route.name === 'settings'}
      {#if settingsComp}
        {@const Comp = settingsComp}
        <Comp section={route.section} />
      {/if}
    {:else if route.name === 'login'}
      <LibraryGate lib={null}><BrowseSkeleton label={t('libstate.loading')} /></LibraryGate>
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
  .boot-error { height: 100%; display: flex; }
  .not-found { height: 100%; display: flex; align-items: center; justify-content: center; color: var(--muted); }
  .toasts { position: fixed; bottom: 20px; right: 20px; display: flex; flex-direction: column; gap: 8px; z-index: 200; }
  .toast { padding: 10px 16px; border-radius: 8px; background: var(--inverse-bg); color: var(--inverse-ink); font-size: 14px; box-shadow: 0 8px 24px rgba(0,0,0,.2); max-width: 420px; }
  .toast.error { background: var(--danger); color: #fff; }
  @media (max-width: 900px) {
    .toasts { bottom: 84px; left: 12px; right: 12px; }
  }
</style>
