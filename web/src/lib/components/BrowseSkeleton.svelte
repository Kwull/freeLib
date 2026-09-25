<script lang="ts">
  // Placeholder panes while the library list (or a name list) loads: a list pane of skeleton
  // rows and a centered spinner with an optional progress line.
  import Spinner from './Spinner.svelte';
  import { paneWidth } from '../stores/layout.svelte';

  let { label, detail = null }: { label: string; detail?: string | null } = $props();
  const widths = [62, 48, 71, 55, 66, 43, 58, 69, 51, 64, 46, 60, 53, 67];
</script>

<div class="browse-skel" data-testid="browse-skeleton" aria-busy="true">
  <div class="list" style:--w="{paneWidth('list')}px" aria-hidden="true">
    <div class="head"><span class="sk title"></span><span class="sk box"></span></div>
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
