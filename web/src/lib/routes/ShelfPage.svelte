<script lang="ts">
  import ScopeBooksPage from './ScopeBooksPage.svelte';
  import { shelvesState } from '../stores/shelves.svelte';

  let { lib, id }: { lib: number; id: number } = $props();
  const shelf = $derived(shelvesState.items.find((s) => s.id === id));
</script>

<div class="shelf-page">
  {#if shelf}
    <div class="head">
      <span class="dot" style="background:{shelf.color}"></span>
      <h1>{shelf.name}</h1>
    </div>
  {/if}
  <ScopeBooksPage {lib} scope={{ kind: 'shelf', id }} />
</div>

<style>
  .shelf-page { display: flex; flex-direction: column; flex-grow: 1; min-width: 0; min-height: 0; }
  .head { display: flex; align-items: center; gap: 10px; padding: 16px 24px; border-bottom: 1px solid var(--line); background: var(--surface); }
  .dot { width: 14px; height: 14px; border-radius: 7px; }
  h1 { margin: 0; font-family: var(--font-display); font-size: 20px; }
  .shelf-page :global(.scope-page) { flex-grow: 1; }
</style>
