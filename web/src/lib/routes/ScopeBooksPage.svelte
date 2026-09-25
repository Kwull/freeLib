<script lang="ts">
  // Shared layout for scopes with no left-hand browse list: New arrivals, a Shelf.
  import BooksPane from '../components/BooksPane.svelte';
  import DetailsPane from '../components/DetailsPane.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import ShelfDialog from '../components/ShelfDialog.svelte';
  import { navigate } from '../router.svelte';

  type Scope = { kind: 'since'; date: string } | { kind: 'shelf'; id: number };

  let { lib, scope }: { lib: number; scope: Scope } = $props();

  let selectedBookId = $state<number | null>(null);
  let sendIds = $state<number[] | null>(null);
  let shelfIds = $state<number[] | null>(null);

  function pickBook(bid: number) {
    selectedBookId = bid;
    if (window.innerWidth < 900) navigate(`/l/${lib}/book/${bid}`);
  }
</script>

<div class="scope-page">
  <BooksPane
    {lib}
    scope={scope.kind === 'since' ? { kind: 'since', date: scope.date, groupable: false } : { kind: 'shelf', id: scope.id, groupable: false }}
    {selectedBookId}
    onPick={pickBook}
    onOpenSend={(ids) => (sendIds = ids)}
    onOpenShelf={(ids) => (shelfIds = ids)}
  />
  <DetailsPane {lib} bookId={selectedBookId} onSend={(ids) => (sendIds = ids)} onAddShelf={(ids) => (shelfIds = ids)} />
</div>

{#if sendIds}<SendDialog {lib} bookIds={sendIds} open={true} onClose={() => (sendIds = null)} />{/if}
{#if shelfIds}<ShelfDialog {lib} bookIds={shelfIds} open={true} onClose={() => (shelfIds = null)} />{/if}

<style>
  .scope-page { display: flex; flex-grow: 1; min-width: 0; min-height: 0; }
</style>
