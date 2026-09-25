<script lang="ts">
  import Dialog from './Dialog.svelte';
  import { shelvesState, setBooksOnShelf, createShelf } from '../stores/shelves.svelte';
  import { t } from '../i18n';

  let { lib, bookIds, open, onClose }: { lib: number; bookIds: number[]; open: boolean; onClose: () => void } = $props();

  let newName = $state('');
  const colors = ['#1F5F5B', '#B8741A', '#6B4E8A', '#2E7D4F', '#B3261E'];

  async function toggleShelf(shelfId: number, on: boolean) {
    await setBooksOnShelf(shelfId, lib, bookIds, on);
  }

  async function addNew() {
    if (!newName.trim()) return;
    const color = colors[shelvesState.items.length % colors.length];
    const shelf = await createShelf(newName.trim(), color);
    newName = '';
    await setBooksOnShelf(shelf.id, lib, bookIds, true);
  }
</script>

<Dialog {open} titleId="shelf-title" title={t('selection.shelf')} {onClose} width={420}>
  <div class="list">
    {#each shelvesState.items as s (s.id)}
      <label class="row">
        <input type="checkbox" onchange={(e) => toggleShelf(s.id, (e.target as HTMLInputElement).checked)} />
        <span class="dot" style="background:{s.color}"></span>
        <span class="name">{s.name}</span>
        <span class="count">{s.count}</span>
      </label>
    {/each}
  </div>
  <div class="new">
    <input type="text" placeholder={t('shelves.add')} bind:value={newName} onkeydown={(e) => e.key === 'Enter' && addNew()} />
    <button type="button" onclick={addNew}>{t('common.save')}</button>
  </div>
  <div class="footer">
    <button type="button" onclick={onClose}>{t('common.close')}</button>
  </div>
</Dialog>

<style>
  .list { padding: 8px 24px 0; display: flex; flex-direction: column; gap: 4px; max-height: 300px; overflow-y: auto; }
  .row { display: flex; align-items: center; gap: 10px; padding: 8px 4px; font-size: 14px; }
  .row input { width: 16px; height: 16px; accent-color: var(--accent); }
  .dot { width: 10px; height: 10px; border-radius: 5px; flex-shrink: 0; }
  .name { flex-grow: 1; }
  .count { color: var(--muted); font-size: 12px; }
  .new { padding: 12px 24px 0; display: flex; gap: 8px; }
  .new input { flex-grow: 1; height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 14px; }
  .new button, .footer button { height: 36px; padding: 0 14px; border-radius: 6px; border: 1px solid var(--border); background: var(--surface); font-size: 14px; }
  .footer { padding: 16px 24px 24px; display: flex; justify-content: flex-end; }
</style>
