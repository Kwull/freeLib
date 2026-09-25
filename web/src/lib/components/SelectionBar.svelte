<script lang="ts">
  import Icon from './Icon.svelte';
  import { t } from '../i18n';
  import { defaultDevice, deviceVerb, deviceCaption } from '../stores/devices.svelte';

  let {
    count, onSend, onDownload, onShelf, onClear,
  }: { count: number; onSend: () => void; onDownload: () => void; onShelf: () => void; onClear: () => void } = $props();

  let menuOpen = $state(false);
  const device = $derived(defaultDevice());
  // the plain verb of the default device (first in the user's order); the dialog lets the
  // user pick another device
  const verb = $derived(device ? t(`device.action.${deviceVerb(device)}`) : t('selection.sendTo'));
  const icon = $derived(device && deviceVerb(device) !== 'send' ? 'download' : 'send');

  function act(fn: () => void) {
    menuOpen = false;
    fn();
  }
</script>

{#if count > 0}
  <div role="region" aria-label="Selection" class="bar">
    <span class="count"><b>{count}</b> {t('selection.selected')}</span>
    <button type="button" class="primary desktop-only" data-testid="selection-send" title={device ? deviceCaption(device) : undefined} onclick={onSend}><Icon name={icon} size={16} />{verb}…</button>
    <button type="button" class="ghost desktop-only" onclick={onDownload}><Icon name="download" size={16} />{t('selection.download')}</button>
    <button type="button" class="ghost desktop-only" onclick={onShelf}><Icon name="shelves" size={16} />{t('selection.shelf')}</button>

    <button type="button" class="primary phone-only" title={device ? deviceCaption(device) : undefined} onclick={onSend}>
      <Icon name={icon} size={16} />
      {verb}…
    </button>
    <div class="more phone-only">
      <button type="button" class="icon" aria-label="More" aria-haspopup="true" aria-expanded={menuOpen} onclick={() => (menuOpen = !menuOpen)}>⋯</button>
      {#if menuOpen}
        <div class="menu" role="menu">
          <button type="button" role="menuitem" onclick={() => act(onDownload)}><Icon name="download" size={16} />{t('selection.download')}</button>
          <button type="button" role="menuitem" onclick={() => act(onShelf)}><Icon name="shelves" size={16} />{t('selection.shelf')}</button>
          <button type="button" role="menuitem" onclick={() => act(onClear)}><Icon name="close" size={16} />{t('selection.clear')}</button>
        </div>
      {/if}
    </div>

    <button type="button" class="icon desktop-only" aria-label={t('selection.clear')} onclick={onClear}><Icon name="close" size={16} /></button>
  </div>
{/if}

<style>
  .bar {
    position: absolute; left: 50%; bottom: 20px; transform: translateX(-50%);
    display: flex; align-items: center; gap: 6px; padding: 6px 6px 6px 16px;
    border-radius: 10px; background: var(--inverse-bg); color: var(--inverse-ink); font-size: 14px; white-space: nowrap;
    z-index: 5; box-shadow: 0 8px 24px rgba(0,0,0,.25);
  }
  .count { margin-right: 8px; }
  button { display: flex; align-items: center; gap: 6px; height: 36px; border: none; border-radius: 7px; font-size: 14px; color: inherit; background: transparent; }
  button:hover { background: var(--inverse-hover); }
  button.primary { padding: 0 14px; background: var(--accent); color: #fff; font-weight: 500; }
  button.primary:hover { background: var(--accent-hover); }
  button.ghost { padding: 0 12px; }
  button.icon { width: 36px; padding: 0; justify-content: center; }
  .phone-only { display: none; }
  .more { position: relative; }
  .menu {
    position: absolute; bottom: 44px; left: 50%; transform: translateX(-50%);
    display: flex; flex-direction: column; gap: 2px; background: var(--surface); border: 1px solid var(--line);
    border-radius: 8px; box-shadow: 0 8px 24px rgba(0,0,0,.25); padding: 6px; min-width: 160px;
  }
  .menu button { color: var(--ink); justify-content: flex-start; padding: 0 10px; }
  .menu button:hover { background: var(--surface-hover); }
  @media (max-width: 900px) {
    .desktop-only { display: none; }
    .phone-only { display: flex; }
    .bar { bottom: 72px; left: 12px; right: 12px; transform: none; width: auto; justify-content: space-between; }
    .count { display: none; }
  }
</style>
