<script lang="ts">
  import ScopeBooksPage from './ScopeBooksPage.svelte';
  import { t } from '../i18n';

  let { lib }: { lib: number } = $props();

  function monthsAgo(n: number): string {
    const d = new Date();
    d.setMonth(d.getMonth() - n);
    return d.toISOString().slice(0, 10);
  }

  let since = $state(monthsAgo(1));
</script>

<div class="new-arrivals">
  <div class="head">
    <h1>{t('newArrivals.title')}</h1>
    <label>{t('newArrivals.since')}
      <input type="date" bind:value={since} />
    </label>
  </div>
  {#key since}
    <ScopeBooksPage {lib} scope={{ kind: 'since', date: since }} />
  {/key}
</div>

<style>
  .new-arrivals { display: flex; flex-direction: column; flex-grow: 1; min-width: 0; min-height: 0; }
  .head { display: flex; align-items: center; gap: 16px; padding: 16px 24px; border-bottom: 1px solid var(--line); background: var(--surface); }
  h1 { margin: 0; font-family: var(--font-display); font-size: 20px; flex-grow: 1; }
  .head label { display: flex; align-items: center; gap: 8px; font-size: 13px; color: var(--muted); }
  .head input { height: 32px; padding: 0 8px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; }
  .new-arrivals :global(.scope-page) { flex-grow: 1; }
  @media (max-width: 900px) {
    .head { flex-wrap: wrap; padding: 12px 16px; gap: 8px 16px; }
    h1 { flex-basis: 100%; font-size: 18px; }
  }
</style>
