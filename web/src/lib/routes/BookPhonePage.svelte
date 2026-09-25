<script lang="ts">
  import DetailsPane from '../components/DetailsPane.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import ShelfDialog from '../components/ShelfDialog.svelte';
  import Icon from '../components/Icon.svelte';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';

  let { lib, id }: { lib: number; id: number } = $props();
  let send = $state<{ ids: number[]; device?: number } | null>(null);
  let shelfIds = $state<number[] | null>(null);
</script>

<div class="book-phone">
  <header>
    <button type="button" aria-label={t('common.back')} onclick={() => (history.length > 1 ? history.back() : navigate(`/l/${lib}/authors`))}>
      <Icon name="chevronLeft" size={20} strokeWidth={2} />
    </button>
  </header>
  <DetailsPane {lib} bookId={id} standalone onSend={(ids, device) => (send = { ids, device })} onAddShelf={(ids) => (shelfIds = ids)} />
</div>

{#if send}<SendDialog {lib} bookIds={send.ids} device={send.device} open={true} onClose={() => (send = null)} />{/if}
{#if shelfIds}<ShelfDialog {lib} bookIds={shelfIds} open={true} onClose={() => (shelfIds = null)} />{/if}

<style>
  .book-phone { display: flex; flex-direction: column; flex-grow: 1; min-height: 0; background: var(--surface-alt); }
  header { display: flex; align-items: center; gap: 4px; padding: 10px 8px; border-bottom: 1px solid var(--line); flex-shrink: 0; }
  header button { width: 44px; height: 44px; border: none; background: transparent; border-radius: 8px; display: flex; align-items: center; justify-content: center; color: var(--ink); }
  .book-phone :global(aside.details) { width: 100%; border-left: none; }
</style>
