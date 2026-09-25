<script lang="ts">
  import ScopeBooksPage from './ScopeBooksPage.svelte';
  import { t, tn } from '../i18n';
  import { getPref, setPref } from '../stores/prefs.svelte';

  let { lib }: { lib: number } = $props();

  function daysAgo(n: number): string {
    const d = new Date();
    d.setDate(d.getDate() - n);
    return d.toISOString().slice(0, 10);
  }

  const PRESETS = [
    { key: 'week', days: 7 },
    { key: 'month', days: 30 },
    { key: 'quarter', days: 91 },
    { key: 'year', days: 365 },
  ] as const;
  type PresetKey = (typeof PRESETS)[number]['key'] | 'custom';

  let preset = $state<PresetKey>(getPref<PresetKey>('newArrivals', 'month'));
  let custom = $state(daysAgo(30));
  const since = $derived(preset === 'custom' ? custom : daysAgo(PRESETS.find((p) => p.key === preset)?.days ?? 30));
  let count = $state<number | null>(null);

  function choose(p: PresetKey) {
    preset = p;
    setPref('newArrivals', p === 'custom' ? 'month' : p);
  }
</script>

<div class="new-arrivals">
  <div class="head">
    <h1>{t('newArrivals.title')}</h1>
    {#if count !== null}<span class="count">{tn('browse.booksCount', count)}</span>{/if}
    <div class="grow"></div>
    <div class="presets" role="group" aria-label={t('newArrivals.since')}>
      {#each PRESETS as p (p.key)}
        <button type="button" class:on={preset === p.key} aria-pressed={preset === p.key} onclick={() => choose(p.key)}>{t(`newArrivals.${p.key}`)}</button>
      {/each}
      <label class:on={preset === 'custom'}>
        <span>{t('newArrivals.since')}</span>
        <input type="date" value={since} max={daysAgo(0)} onchange={(e) => { custom = (e.currentTarget as HTMLInputElement).value || daysAgo(30); choose('custom'); }} />
      </label>
    </div>
  </div>
  {#key since}
    <ScopeBooksPage {lib} scope={{ kind: 'since', date: since }} onCounts={(c) => (count = c.books)} />
  {/key}
</div>

<style>
  .new-arrivals { display: flex; flex-direction: column; flex-grow: 1; min-width: 0; min-height: 0; }
  .head { display: flex; align-items: center; gap: 12px 16px; padding: 14px 24px; border-bottom: 1px solid var(--line); background: var(--surface); flex-wrap: wrap; }
  h1 { margin: 0; font-family: var(--font-display); font-size: 20px; }
  .count { color: var(--muted); font-size: 13px; }
  .grow { flex-grow: 1; }
  .presets { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .presets button, .presets label {
    display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 10px; border: 1px solid var(--border);
    border-radius: 15px; background: var(--surface); color: var(--muted-2); font-size: 13px;
  }
  .presets .on { background: var(--accent-soft); border-color: var(--accent); color: var(--accent-soft-ink); }
  .presets input { border: none; background: transparent; font: inherit; font-size: 13px; color: inherit; height: 26px; }
  .new-arrivals :global(.scope-page) { flex-grow: 1; }
  @media (max-width: 900px) {
    .head { padding: 10px 12px; }
    h1 { font-size: 18px; }
    .grow { display: none; }
    .presets { flex-basis: 100%; overflow-x: auto; flex-wrap: nowrap; }
    .presets button, .presets label { flex-shrink: 0; }
  }
</style>
