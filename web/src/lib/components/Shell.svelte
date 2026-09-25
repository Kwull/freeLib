<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import GlobalSearch from './GlobalSearch.svelte';
  import ActivityPanel from './ActivityPanel.svelte';
  import PhoneNav from './PhoneNav.svelte';
  import { t, i18nState, setLang } from '../i18n';
  import { librariesState, currentLibrary, setCurrentLibrary, isBrowsable } from '../stores/libraries.svelte';
  import type { Library } from '../api/types';
  import { jobsState, toggleActivity, runningCount } from '../stores/jobs.svelte';
  import { sessionState, logout } from '../stores/session.svelte';
  import { themeState, setTheme } from '../stores/theme.svelte';
  import { navigate, currentRoute } from '../router.svelte';
  import { shelvesState } from '../stores/shelves.svelte';
  import Splitter from './Splitter.svelte';
  import { PANE_LIMITS, paneWidth, setPaneWidth } from '../stores/layout.svelte';

  let { children }: { children: Snippet } = $props();

  let libMenuOpen = $state(false);
  let accountMenuOpen = $state(false);
  const lib = $derived(currentLibrary());
  const route = $derived(currentRoute());

  let liveNav = $state<number | null>(null);
  const navWidth = $derived(liveNav ?? paneWidth('nav'));

  function statusClass(l: Library | null): string {
    if (!l) return '';
    if (l.status.state === 'importing') return 'importing';
    if (l.status.state === 'error' || !isBrowsable(l)) return 'error';
    return 'ok';
  }

  function isActive(name: string): boolean {
    return route.name === name;
  }
</script>

<div class="shell">
  <header class="topbar">
    <a href="/" data-link class="brand">
      <Icon name="library" size={24} strokeWidth={1.8} />
      <span>{t('app.name')}</span>
    </a>
    {#if !librariesState.loaded && !librariesState.error}
      <span class="sk sk-pill" aria-hidden="true"></span>
    {/if}
    {#if librariesState.items.length}
      <div class="lib-switch">
        <button type="button" onclick={() => (libMenuOpen = !libMenuOpen)} aria-haspopup="true" aria-expanded={libMenuOpen}>
          <span class="dot {statusClass(lib)}" title={lib ? t(`libraries.status.${lib.status.state}`) : ''}></span>
          <span class="lib-name">{lib?.name ?? ''}</span>
          <Icon name="chevronDown" size={16} />
        </button>
        {#if libMenuOpen}
          <div class="menu" role="menu">
            {#each librariesState.items as l (l.id)}
              <button
                type="button"
                role="menuitem"
                onclick={() => { setCurrentLibrary(l.id); libMenuOpen = false; navigate(`/l/${l.id}/authors`); }}
              ><span class="dot {statusClass(l)}"></span>{l.name}</button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
    {#if lib && isBrowsable(lib)}<GlobalSearch lib={lib.id} />
    {:else if !librariesState.loaded && !librariesState.error}<span class="sk sk-search" aria-hidden="true"></span>{/if}
    <div class="grow"></div>
    <button type="button" class="icon-btn" aria-label="Activity" onclick={() => toggleActivity()}>
      <Icon name="activity" size={20} strokeWidth={1.8} />
      {#if runningCount() > 0}<span class="badge"></span>{/if}
    </button>
    <div class="account">
      <button type="button" class="avatar" aria-label="Account" onclick={() => (accountMenuOpen = !accountMenuOpen)}>
        {(sessionState.user?.username ?? '?').charAt(0).toUpperCase()}
      </button>
      {#if accountMenuOpen}
        <div class="menu" role="menu">
          <div class="menu-section">{t('account.theme')}</div>
          <div class="seg">
            <button type="button" class:on={themeState.value === 'light'} onclick={() => setTheme('light')}>{t('theme.light')}</button>
            <button type="button" class:on={themeState.value === 'dark'} onclick={() => setTheme('dark')}>{t('theme.dark')}</button>
            <button type="button" class:on={themeState.value === 'system'} onclick={() => setTheme('system')}>{t('theme.system')}</button>
          </div>
          <div class="menu-section">{t('account.language')}</div>
          <div class="seg">
            <button type="button" class:on={i18nState.lang === 'en'} onclick={() => setLang('en')}>EN</button>
            <button type="button" class:on={i18nState.lang === 'ru'} onclick={() => setLang('ru')}>RU</button>
            <button type="button" class:on={i18nState.lang === 'uk'} onclick={() => setLang('uk')}>UK</button>
          </div>
          {#if !sessionState.openMode}
            <button type="button" class="menu-item" onclick={() => { accountMenuOpen = false; navigate('/settings/account'); }}>
              <Icon name="user" size={16} />{t('account.menu')}
            </button>
          {/if}
          <button type="button" class="menu-item" onclick={() => { accountMenuOpen = false; navigate('/settings'); }}>
            <Icon name="settings" size={16} />{t('nav.settings')}
          </button>
          {#if !sessionState.openMode}
            <button type="button" class="menu-item" onclick={() => { logout(); navigate('/login'); }}>
              <Icon name="logout" size={16} />{t('account.logout')}
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </header>

  <div class="body">
    {#if lib}
      <nav aria-label="Main" class="sidenav" style:--w="{navWidth}px">
        <a class="nav" href="/l/{lib.id}/new" data-link aria-current={isActive('new') ? 'page' : undefined} class:active={isActive('new')}>
          <Icon name="newArrivals" size={18} /><span class="label">{t('nav.newArrivals')}</span>
          {#if lib.newSinceLastVisit > 0}<span class="count">{lib.newSinceLastVisit}</span>{/if}
        </a>
        <a class="nav" href="/l/{lib.id}/authors" data-link aria-current={isActive('authors') ? 'page' : undefined} class:active={isActive('authors')}>
          <Icon name="authors" size={18} /><span class="label">{t('nav.authors')}</span>
        </a>
        <a class="nav" href="/l/{lib.id}/series" data-link aria-current={isActive('series') ? 'page' : undefined} class:active={isActive('series')}>
          <Icon name="series" size={18} /><span class="label">{t('nav.series')}</span>
        </a>
        <a class="nav" href="/l/{lib.id}/genres" data-link aria-current={isActive('genres') ? 'page' : undefined} class:active={isActive('genres')}>
          <Icon name="genres" size={18} /><span class="label">{t('nav.genres')}</span>
        </a>
        <a class="nav" href="/l/{lib.id}/shelves" data-link aria-current={isActive('shelvesIndex') ? 'page' : undefined} class:active={isActive('shelvesIndex')}>
          <Icon name="shelves" size={18} /><span class="label">{t('nav.shelves')}</span>
        </a>
        <div class="sep"></div>
        {#if shelvesState.items.length}<div class="section-label">{t('nav.myShelves')}</div>{/if}
        {#each shelvesState.items as s (s.id)}
          {@const on = route.name === 'shelf' && route.id === s.id}
          <a class="nav" href="/l/{lib.id}/shelves/{s.id}" data-link class:active={on} aria-current={on ? 'page' : undefined}>
            <span class="dot" style="background:{s.color}"></span><span class="label">{s.name}</span>
            <span class="count">{s.count}</span>
          </a>
        {/each}
        <div class="grow"></div>
        <a class="nav" href="/libraries" data-link aria-current={isActive('libraries') ? 'page' : undefined} class:active={isActive('libraries')}>
          <Icon name="library" size={18} /><span class="label">{t('nav.libraries')}</span>
        </a>
        <a class="nav" href="/settings" data-link aria-current={isActive('settings') ? 'page' : undefined} class:active={isActive('settings')}>
          <Icon name="settings" size={18} /><span class="label">{t('nav.settings')}</span>
        </a>
      </nav>
      <Splitter
        value={navWidth}
        min={PANE_LIMITS.nav.min}
        max={PANE_LIMITS.nav.max}
        label={t('layout.resizeNav')}
        onInput={(v) => (liveNav = v)}
        onCommit={(v) => { setPaneWidth('nav', v); liveNav = null; }}
        onReset={() => { setPaneWidth('nav', null); liveNav = null; }}
      />
    {:else if !librariesState.loaded && !librariesState.error}
      <nav aria-label="Main" class="sidenav" style:--w="{navWidth}px" aria-busy="true">
        {#each [70, 55, 50, 58, 62] as w, i (i)}
          <div class="nav sk-row" aria-hidden="true"><span class="sk sk-icon"></span><span class="sk" style:width="{w}%"></span></div>
        {/each}
      </nav>
    {:else}
      <nav aria-label="Main" class="sidenav" style:--w="{navWidth}px">
        <div class="grow"></div>
        <a class="nav" href="/libraries" data-link aria-current={isActive('libraries') ? 'page' : undefined} class:active={isActive('libraries')}>
          <Icon name="library" size={18} /><span class="label">{t('nav.libraries')}</span>
        </a>
        <a class="nav" href="/settings" data-link aria-current={isActive('settings') ? 'page' : undefined} class:active={isActive('settings')}>
          <Icon name="settings" size={18} /><span class="label">{t('nav.settings')}</span>
        </a>
      </nav>
    {/if}
    <div class="content">
      {@render children()}
    </div>
  </div>

  {#if lib}
    <div class="phone-nav-wrap"><PhoneNav lib={lib.id} /></div>
  {/if}

  <ActivityPanel />
</div>

<style>
  .shell { height: 100%; display: flex; flex-direction: column; background: var(--page); }
  .topbar {
    height: 56px; flex-shrink: 0; display: flex; align-items: center; gap: 16px; padding: 0 16px 0 20px;
    background: var(--surface); border-bottom: 1px solid var(--line);
  }
  .brand { display: flex; align-items: center; gap: 10px; width: 172px; color: var(--ink); text-decoration: none; flex-shrink: 0; }
  .brand span { font-family: var(--font-display); font-size: 20px; font-weight: 600; }
  .lib-switch { position: relative; flex-shrink: 0; }
  .lib-switch > button {
    display: flex; align-items: center; gap: 8px; height: 36px; padding: 0 12px; border: 1px solid var(--border);
    border-radius: 8px; background: var(--surface); font-size: 14px; color: var(--ink);
  }
  .lib-name { max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .dot { width: 8px; height: 8px; border-radius: 4px; flex-shrink: 0; }
  .dot.ok { background: #2E7D4F; }
  .dot.importing { background: var(--amber); animation: pulse 1.2s ease-in-out infinite; }
  .dot.error { background: var(--danger); }
  .lib-switch .menu button { display: flex; align-items: center; gap: 8px; }
  .sk { display: block; height: 10px; border-radius: 5px; background: var(--surface-hover); animation: pulse 1.2s ease-in-out infinite; }
  .sk-pill { width: 150px; height: 36px; border-radius: 8px; flex-shrink: 0; }
  .sk-search { width: min(420px, 30vw); height: 36px; border-radius: 8px; }
  .sk-icon { width: 18px; height: 18px; border-radius: 5px; flex-shrink: 0; }
  .sk-row { gap: 12px; }
  .sk-row:hover { background: none; }
  @keyframes pulse { 50% { opacity: .45; } }
  .grow { flex-grow: 1; }
  .icon-btn {
    position: relative; display: flex; align-items: center; justify-content: center; width: 44px; height: 44px;
    border-radius: 8px; color: var(--muted-2); border: none; background: transparent; flex-shrink: 0;
  }
  .icon-btn:hover { background: var(--surface-hover); }
  .badge { position: absolute; top: 8px; right: 8px; width: 8px; height: 8px; border-radius: 4px; background: var(--amber); }
  .account { position: relative; flex-shrink: 0; }
  .avatar { width: 36px; height: 36px; border-radius: 18px; border: none; background: var(--accent); color: #fff; font-size: 14px; font-weight: 600; }
  .menu {
    position: absolute; top: 46px; right: 0; background: var(--surface); border: 1px solid var(--line); border-radius: 10px;
    box-shadow: 0 12px 32px rgba(0,0,0,.18); padding: 8px; z-index: 60; min-width: 200px; display: flex; flex-direction: column; gap: 4px;
  }
  .lib-switch .menu { left: 0; right: auto; min-width: 220px; }
  .lib-switch .menu button { text-align: left; padding: 8px 10px; border-radius: 6px; border: none; background: transparent; font-size: 14px; }
  .lib-switch .menu button:hover { background: var(--surface-hover); }
  .menu-section { font-size: 11px; font-weight: 600; color: var(--muted); padding: 6px 6px 2px; letter-spacing: .04em; }
  .seg { display: flex; gap: 4px; padding: 0 4px 4px; }
  .seg button { flex: 1; height: 30px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font-size: 12px; }
  .seg button.on { background: var(--accent-soft); border-color: var(--accent); color: var(--accent-soft-ink); }
  .menu-item { display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-radius: 6px; border: none; background: transparent; font-size: 14px; text-align: left; }
  .menu-item:hover { background: var(--surface-hover); }
  .body { flex-grow: 1; display: flex; min-height: 0; }
  .sidenav { flex: 0 0 var(--w, 208px); min-width: 0; display: flex; flex-direction: column; gap: 2px; padding: 16px 12px; border-right: 1px solid var(--line); overflow-y: auto; }
  .nav { display: flex; align-items: center; gap: 12px; height: 40px; padding: 0 12px; border-radius: 8px; color: var(--muted-2); font-size: 14px; text-decoration: none; }
  .nav:hover { background: var(--surface-hover); text-decoration: none; color: var(--ink); }
  .nav.active { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 500; }
  .nav .label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .nav .dot { flex-shrink: 0; }
  .nav .count { margin-left: auto; flex-shrink: 0; font-size: 12px; color: var(--muted); }
  .sep { height: 1px; background: var(--line); margin: 12px 4px; }
  .section-label { padding: 0 12px 6px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; }
  .content { flex-grow: 1; min-width: 0; display: flex; min-height: 0; }
  .phone-nav-wrap { display: none; }

  @media (max-width: 900px) {
    .sidenav { display: none; }
    .phone-nav-wrap { display: block; }
    .brand span { display: none; }
    .brand { width: auto; }
    .topbar { gap: 8px; padding: 0 8px 0 12px; }
    .lib-name { max-width: 42vw; }
    .lib-switch button { padding: 0 8px; }
    .topbar :global(.wrap), .sk-search { display: none; }
  }
</style>
