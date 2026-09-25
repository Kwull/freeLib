<script lang="ts">
  // Open Library rating in lists: a star, the average and the vote count.
  import Icon from './Icon.svelte';
  import { t, tn } from '../i18n';
  import { formatAvg } from '../utils/ratings';

  let { value }: { value: { avg: number; votes: number } | null } = $props();
  /** 2104 → "2.1k" */
  const compact = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(n >= 10000 ? 0 : 1)}k` : String(n));
</script>

{#if value}
  <span class="ext" title={t('ratings.extTooltip', { avg: formatAvg(value.avg), votes: tn('ratings.votes', value.votes) })}>
    <Icon name="star" size={12} strokeWidth={1.6} class="on" />
    <span class="avg">{formatAvg(value.avg)}</span>
    <span class="votes">({compact(value.votes)})</span>
  </span>
{/if}

<style>
  .ext { display: inline-flex; align-items: center; gap: 3px; font-size: 12px; white-space: nowrap; font-variant-numeric: tabular-nums; }
  .ext :global(svg.on) { fill: var(--sky, #3b82c4); stroke: var(--sky, #3b82c4); }
  .avg { color: var(--ink); }
  .votes { color: var(--muted); }
</style>
