<script lang="ts">
  // A centered card for a whole pane: loading, importing, empty and error states.
  // `progress` is 0..1 for a determinate bar, `null` for an indeterminate one, undefined for none.
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import Spinner from './Spinner.svelte';

  let {
    tone = 'info',
    icon,
    title,
    text,
    detail,
    progress,
    step,
    busy = false,
    testid,
    actions,
    children,
  }: {
    tone?: 'info' | 'error';
    icon?: string;
    title: string;
    text?: string;
    /** Monospace detail, e.g. the server's error message. */
    detail?: string | null;
    progress?: number | null;
    step?: string | null;
    busy?: boolean;
    testid?: string;
    actions?: Snippet;
    children?: Snippet;
  } = $props();

  const pct = $derived(typeof progress === 'number' ? Math.max(0, Math.min(1, progress)) : null);
</script>

<div class="wrap" data-testid={testid}>
  <div class="card" class:error={tone === 'error'} role={tone === 'error' ? 'alert' : 'status'} aria-live="polite">
    {#if busy}
      <div class="badge"><Spinner size={30} /></div>
    {:else if icon}
      <div class="badge"><Icon name={icon} size={26} strokeWidth={1.8} /></div>
    {/if}
    <h2>{title}</h2>
    {#if text}<p class="text">{text}</p>{/if}
    {#if progress !== undefined}
      <div class="progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct === null ? undefined : Math.round(pct * 100)}>
        <div class="bar" class:indeterminate={pct === null} style:width={pct === null ? undefined : `${pct * 100}%`}></div>
      </div>
      <div class="step">
        <span class="step-text">{step ?? ''}</span>
        {#if pct !== null}<span class="pct">{Math.floor(pct * 100)}%</span>{/if}
      </div>
    {/if}
    {#if detail}<pre class="detail">{detail}</pre>{/if}
    {#if children}{@render children()}{/if}
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </div>
</div>

<style>
  .wrap { flex-grow: 1; display: flex; align-items: center; justify-content: center; padding: 24px 16px; min-width: 0; min-height: 0; overflow-y: auto; background: var(--page); }
  .card {
    width: 100%; max-width: 460px; display: flex; flex-direction: column; align-items: center; gap: 10px; text-align: center;
    padding: 32px 28px 28px; background: var(--surface); border: 1px solid var(--line); border-radius: 14px;
    box-shadow: 0 12px 32px rgba(0, 0, 0, .06);
  }
  .badge { width: 56px; height: 56px; border-radius: 28px; display: flex; align-items: center; justify-content: center; background: var(--accent-soft); color: var(--accent-soft-ink); margin-bottom: 4px; }
  .card.error .badge { background: color-mix(in srgb, var(--danger) 14%, transparent); color: var(--danger); }
  h2 { margin: 0; font-family: var(--font-display); font-size: 21px; font-weight: 600; color: var(--ink); }
  .text { margin: 0; color: var(--muted); font-size: 14px; line-height: 1.5; max-width: 380px; }
  .progress { width: 100%; height: 8px; border-radius: 4px; background: var(--surface-hover); overflow: hidden; margin-top: 10px; position: relative; }
  .bar { height: 100%; background: var(--accent); border-radius: 4px; transition: width .4s ease; }
  .bar.indeterminate { position: absolute; width: 35%; animation: slide 1.3s ease-in-out infinite; }
  @keyframes slide { from { left: -35%; } to { left: 100%; } }
  .step { width: 100%; display: flex; justify-content: space-between; gap: 12px; font-size: 13px; color: var(--muted); min-height: 18px; }
  .step-text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: left; }
  .pct { font-variant-numeric: tabular-nums; color: var(--ink); font-weight: 500; }
  .detail {
    width: 100%; margin: 4px 0 0; padding: 10px 12px; border-radius: 8px; background: var(--surface-alt); border: 1px solid var(--line-soft);
    font-size: 12px; color: var(--muted-2); white-space: pre-wrap; word-break: break-word; text-align: left; max-height: 160px; overflow: auto;
  }
  .actions { display: flex; flex-wrap: wrap; justify-content: center; gap: 8px; margin-top: 10px; }
  .actions :global(button), .actions :global(a) {
    display: inline-flex; align-items: center; gap: 8px; height: 38px; padding: 0 16px; border-radius: 8px; font-size: 14px;
    border: 1px solid var(--border); background: var(--surface); color: var(--ink); text-decoration: none; cursor: pointer;
  }
  .actions :global(.primary) { background: var(--accent); border-color: var(--accent); color: #fff; }
  .actions :global(button:hover), .actions :global(a:hover) { background: var(--surface-hover); text-decoration: none; }
  .actions :global(.primary:hover) { background: var(--accent-hover); color: #fff; }
  @media (prefers-reduced-motion: reduce) { .bar.indeterminate { animation-duration: 3s; } }
</style>
