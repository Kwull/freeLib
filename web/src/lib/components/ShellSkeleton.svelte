<script lang="ts">
  // The app's frame while the session loads: the real top bar's brand with skeleton controls,
  // a skeleton navigation and skeleton panes — never a bare "Loading…".
  import Icon from './Icon.svelte';
  import BrowseSkeleton from './BrowseSkeleton.svelte';
  import { t } from '../i18n';
  import { paneWidth } from '../stores/layout.svelte';

  let { label }: { label: string } = $props();
</script>

<div class="shell-skel" data-testid="shell-skeleton">
  <header class="topbar">
    <span class="brand"><Icon name="library" size={24} strokeWidth={1.8} /><span>{t('app.name')}</span></span>
    <span class="sk pill"></span>
    <span class="sk search"></span>
    <span class="grow"></span>
    <span class="sk dot"></span>
  </header>
  <div class="body">
    <nav class="sidenav" style:--w="{paneWidth('nav')}px" aria-hidden="true">
      {#each [70, 55, 50, 58, 62] as w, i (i)}
        <div class="nav-row"><span class="sk icon"></span><span class="sk" style:width="{w}%"></span></div>
      {/each}
    </nav>
    <BrowseSkeleton {label} />
  </div>
</div>

<style>
  .shell-skel { height: 100%; display: flex; flex-direction: column; background: var(--page); }
  .topbar { height: 56px; flex-shrink: 0; display: flex; align-items: center; gap: 16px; padding: 0 16px 0 20px; background: var(--surface); border-bottom: 1px solid var(--line); }
  .brand { display: flex; align-items: center; gap: 10px; width: 172px; color: var(--ink); flex-shrink: 0; }
  .brand span { font-family: var(--font-display); font-size: 20px; font-weight: 600; }
  .sk { display: block; height: 10px; border-radius: 5px; background: var(--surface-hover); animation: pulse 1.2s ease-in-out infinite; }
  .pill { width: 150px; height: 36px; border-radius: 8px; flex-shrink: 0; }
  .search { width: min(420px, 30vw); height: 36px; border-radius: 8px; }
  .dot { width: 36px; height: 36px; border-radius: 18px; flex-shrink: 0; }
  .grow { flex-grow: 1; }
  .body { flex-grow: 1; display: flex; min-height: 0; }
  .sidenav { flex: 0 0 var(--w, 208px); padding: 20px 20px; border-right: 1px solid var(--line); display: flex; flex-direction: column; gap: 6px; }
  .nav-row { display: flex; align-items: center; gap: 12px; height: 34px; }
  .icon { width: 18px; height: 18px; border-radius: 5px; flex-shrink: 0; }
  @keyframes pulse { 50% { opacity: .45; } }
  @media (max-width: 900px) {
    .sidenav, .search { display: none; }
    .brand span { display: none; }
    .brand { width: auto; }
    .topbar { gap: 8px; padding: 0 8px 0 12px; }
  }
</style>
