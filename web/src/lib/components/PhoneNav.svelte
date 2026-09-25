<script lang="ts">
  import Icon from './Icon.svelte';
  import { t } from '../i18n';
  import { currentRoute } from '../router.svelte';

  let { lib }: { lib: number } = $props();
  const route = $derived(currentRoute());

  const tabs = $derived([
    { name: 'new', href: `/l/${lib}/new`, icon: 'newArrivals', label: t('phone.tabs.new'), active: route.name === 'new' },
    { name: 'authors', href: `/l/${lib}/authors`, icon: 'authors', label: t('phone.tabs.authors'), active: route.name === 'authors' || route.name === 'series' },
    { name: 'search', href: `/l/${lib}/search`, icon: 'search', label: t('phone.tabs.search'), active: route.name === 'search' },
    { name: 'genres', href: `/l/${lib}/genres`, icon: 'genres', label: t('phone.tabs.genres'), active: route.name === 'genres' },
    { name: 'libraries', href: '/libraries', icon: 'library', label: t('phone.tabs.libraries'), active: route.name === 'libraries' },
  ]);
</script>

<nav aria-label="Main" class="phone-nav">
  {#each tabs as tb (tb.name)}
    <a class="tab" href={tb.href} data-link aria-current={tb.active ? 'page' : undefined} class:active={tb.active}>
      <Icon name={tb.icon} size={22} strokeWidth={1.8} />
      <span class="lbl">{tb.label}</span>
    </a>
  {/each}
</nav>

<style>
  .phone-nav {
    display: flex; justify-content: space-around; border-top: 1px solid var(--line); padding-bottom: 8px;
    background: var(--surface); flex-shrink: 0;
  }
  .tab {
    display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px;
    width: 78px; height: 56px; padding: 0 4px; font-size: 11px; color: var(--muted); text-decoration: none;
  }
  .tab .lbl { max-width: 100%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tab.active { color: var(--accent); font-weight: 600; }
</style>
