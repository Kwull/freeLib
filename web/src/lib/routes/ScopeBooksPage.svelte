<script lang="ts">
  // Shared layout for scopes with no left-hand browse list: New arrivals, a Shelf.
  import BooksPane from '../components/BooksPane.svelte';
  import DetailsPane from '../components/DetailsPane.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import ShelfDialog from '../components/ShelfDialog.svelte';
  import { navigate } from '../router.svelte';

  type Scope = { kind: 'since'; date: string } | { kind: 'shelf'; id: number };

  let { lib, scope, onCounts }: { lib: number; scope: Scope; onCounts?: (c: { books: number }) => void } = $props();

  let selectedBookId = $state<number | null>(null);
  let send = $state<{ ids: number[]; device?: number } | null>(null);
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
    onOpenSend={(ids) => (send = { ids })}
    onOpenShelf={(ids) => (shelfIds = ids)}
    {onCounts}
  />
  <DetailsPane {lib} bookId={selectedBookId} onSend={(ids, device) => (send = { ids, device })} onAddShelf={(ids) => (shelfIds = ids)} />
</div>

{#if send}<SendDialog {lib} bookIds={send.ids} device={send.device} open={true} onClose={() => (send = null)} />{/if}
{#if shelfIds}<ShelfDialog {lib} bookIds={shelfIds} open={true} onClose={() => (shelfIds = null)} />{/if}

<style>
  .scope-page { display: flex; flex-grow: 1; min-width: 0; min-height: 0; }
  @media (max-width: 900px) {
    /* Book selection navigates to its own /book/:id route on phone (see pickBook above),
       so the side-by-side details pane would only squeeze the book list; hide it. */
    .scope-page :global(.details) { display: none; }
  }
</style>
