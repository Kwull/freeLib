<script lang="ts">
  import Icon from './Icon.svelte';
  import { t } from '../i18n';

  let {
    count, onSend, onDownload, onShelf, onClear,
  }: { count: number; onSend: () => void; onDownload: () => void; onShelf: () => void; onClear: () => void } = $props();
</script>

{#if count > 0}
  <div role="region" aria-label="Selection" class="bar">
    <span class="count"><b>{count}</b> {t('selection.selected')}</span>
    <button type="button" class="primary" onclick={onSend}><Icon name="send" size={16} />{t('selection.sendTo')}</button>
    <button type="button" class="ghost" onclick={onDownload}><Icon name="download" size={16} />{t('selection.download')}</button>
    <button type="button" class="ghost" onclick={onShelf}><Icon name="shelves" size={16} />{t('selection.shelf')}</button>
    <button type="button" class="icon" aria-label={t('selection.clear')} onclick={onClear}><Icon name="close" size={16} /></button>
  </div>
{/if}

<style>
  .bar {
    position: absolute; left: 50%; bottom: 20px; transform: translateX(-50%);
    display: flex; align-items: center; gap: 6px; padding: 6px 6px 6px 16px;
    border-radius: 10px; background: var(--ink); color: #FFFFFF; font-size: 14px; white-space: nowrap;
    z-index: 5; box-shadow: 0 8px 24px rgba(0,0,0,.25);
  }
  .count { margin-right: 8px; }
  button { display: flex; align-items: center; gap: 6px; height: 36px; border: none; border-radius: 7px; font-size: 14px; color: #FFFFFF; background: transparent; }
  button.primary { padding: 0 14px; background: var(--accent); font-weight: 500; }
  button.ghost { padding: 0 12px; }
  button.icon { width: 36px; padding: 0; justify-content: center; }
</style>
