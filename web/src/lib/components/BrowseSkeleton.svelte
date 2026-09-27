<script lang="ts">
  // Placeholder panes while the library list (or a name list) loads: a list pane of skeleton
  // rows and a centered spinner with an optional progress line. On the authors / series pages
  // the list pane has the real filter box already: what is typed there is kept (nameFilter
  // store) and the real list starts with it, focused.
  import Spinner from './Spinner.svelte';
  import { paneWidth } from '../stores/layout.svelte';
  import { nameFilterState, getNameFilter, setNameFilter } from '../stores/nameFilter.svelte';

  let { label, detail = null, filter = null }: {
    label: string;
    detail?: string | null;
    /** the authors / series filter box: its store key and label */
    filter?: { key: string; label: string } | null;
  } = $props();

  /** the box that had the focus in the previous placeholder (the boot frame) keeps it */
  function takeFocus(el: HTMLInputElement) {
    if (filter && nameFilterState.focusKey === filter.key) {
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    }
  }

  function onBlur(e: FocusEvent) {
    const el = e.currentTarget as HTMLInputElement;
    // the box leaving the page (the real list replaces it) keeps the focus for the real box
    setTimeout(() => { if (el.isConnected && nameFilterState.focusKey === filter?.key) nameFilterState.focusKey = null; }, 0);
  }
  const widths = [62, 48, 71, 55, 66, 43, 58, 69, 51, 64, 46, 60, 53, 67];
</script>

<div class="browse-skel" data-testid="browse-skeleton" aria-busy="true">
  <div class="list" style:--w="{paneWidth('list')}px" aria-hidden={filter ? undefined : 'true'}>
    <div class="head">
      <span class="sk title"></span>
      {#if filter}
        <input class="filter" type="text" use:takeFocus aria-label={filter.label} placeholder={filter.label} data-testid="early-filter"
          value={getNameFilter(filter.key)}
          oninput={(e) => setNameFilter(filter!.key, (e.currentTarget as HTMLInputElement).value)}
          onfocus={() => (nameFilterState.focusKey = filter!.key)} onblur={onBlur} />
      {:else}
        <span class="sk box"></span>
      {/if}
    </div>
    {#each widths as w, i (i)}
      <div class="row"><span class="sk" style:width="{w}%"></span><span class="sk n"></span></div>
    {/each}
  </div>
  <div class="main">
    <Spinner size={32} {label} />
    <p class="label">{label}</p>
    {#if detail}<p class="detail">{detail}</p>{/if}
  </div>
</div>

<style>
  .browse-skel { flex-grow: 1; display: flex; min-width: 0; min-height: 0; }
  .list { flex: 0 1 var(--w, 280px); min-width: 200px; border-right: 1px solid var(--line); background: var(--surface-alt); padding: 14px 12px 8px 16px; display: flex; flex-direction: column; gap: 0; overflow: hidden; }
  .head { display: flex; flex-direction: column; gap: 12px; margin-bottom: 10px; }
  .sk { display: block; height: 10px; border-radius: 5px; background: var(--surface-hover); animation: pulse 1.2s ease-in-out infinite; }
  .title { width: 40%; height: 14px; margin-top: 6px; }
  .box { width: 100%; height: 34px; border-radius: 6px; }
  .filter { width: 100%; height: 34px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 14px; color: var(--ink); outline: none; }
  .filter:focus { border-color: var(--accent); }
  .row { display: flex; align-items: center; justify-content: space-between; height: 36px; padding: 0 10px; gap: 12px; }
  .n { width: 22px; flex-shrink: 0; }
  .main { flex-grow: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 12px; background: var(--surface); padding: 24px; text-align: center; }
  .label { margin: 0; color: var(--muted-2); font-size: 15px; }
  .detail { margin: -4px 0 0; color: var(--muted); font-size: 13px; font-variant-numeric: tabular-nums; }
  @keyframes pulse { 50% { opacity: .45; } }
  @media (max-width: 900px) {
    .list { flex: 1 1 auto; border-right: none; }
    .main { display: none; }
  }
</style>
