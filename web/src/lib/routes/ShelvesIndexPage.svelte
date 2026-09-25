<script lang="ts">
  import { shelvesState, createShelf, deleteShelf } from '../stores/shelves.svelte';
  import Icon from '../components/Icon.svelte';
  import { t } from '../i18n';

  let { lib }: { lib: number } = $props();
  let newName = $state('');
  const colors = ['#1F5F5B', '#B8741A', '#6B4E8A', '#2E7D4F', '#B3261E'];

  async function add() {
    if (!newName.trim()) return;
    await createShelf(newName.trim(), colors[shelvesState.items.length % colors.length]);
    newName = '';
  }
</script>

<main class="page">
  <h1>{t('shelves.title')}</h1>
  {#if !shelvesState.items.length}<p class="hint">{t('shelves.emptyHint')}</p>{/if}
  <div class="grid">
    {#each shelvesState.items as s (s.id)}
      <a class="card" href="/l/{lib}/shelves/{s.id}" data-link>
        <span class="dot" style="background:{s.color}"></span>
        <span class="name">{s.name}</span>
        <span class="count">{s.count}</span>
        <button type="button" class="del" aria-label="{t('common.delete')} {s.name}" onclick={(e) => { e.preventDefault(); e.stopPropagation(); if (confirm(t('shelves.deleteConfirm', { name: s.name }))) deleteShelf(s.id); }}>
          <Icon name="trash" size={14} />
        </button>
      </a>
    {/each}
  </div>
  <div class="new">
    <input type="text" placeholder={t('shelves.add')} bind:value={newName} onkeydown={(e) => e.key === 'Enter' && add()} />
    <button type="button" onclick={add}><Icon name="plus" size={16} />{t('shelves.add')}</button>
  </div>
</main>

<style>
  .page { flex-grow: 1; overflow-y: auto; padding: 24px 28px; background: var(--surface); }
  h1 { margin: 0 0 16px; font-family: var(--font-display); font-size: 24px; }
  .hint { margin: 0 0 16px; max-width: 560px; color: var(--muted); font-size: 14px; line-height: 1.5; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 12px; }
  .card {
    position: relative; display: flex; flex-direction: column; gap: 6px; padding: 16px; border-radius: 10px;
    border: 1px solid var(--line); background: var(--surface-alt); text-decoration: none; color: var(--ink);
  }
  .card:hover { background: var(--surface-hover); text-decoration: none; }
  .dot { width: 14px; height: 14px; border-radius: 7px; }
  .name { font-weight: 600; }
  .count { font-size: 13px; color: var(--muted); }
  .del { position: absolute; top: 10px; right: 10px; width: 26px; height: 26px; border: none; border-radius: 6px; background: transparent; color: var(--muted); display: flex; align-items: center; justify-content: center; }
  .del:hover { background: var(--surface-hover); color: var(--danger); }
  .new { margin-top: 20px; display: flex; gap: 8px; max-width: 320px; }
  .new input { flex-grow: 1; height: 38px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; }
  .new button { white-space: nowrap; display: flex; align-items: center; gap: 6px; height: 38px; padding: 0 14px; border-radius: 6px; border: none; background: var(--accent); color: #fff; font-size: 14px; }
</style>
