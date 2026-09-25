<script lang="ts">
  // Rating filters (my / library / Open Library minimums, "not rated by me", kids age) for the
  // books filter menu and the search facets. Mutates `filters` through onChange.
  import { t } from '../i18n';
  import { KIDS_AGES, type RatingFilters } from '../utils/ratings';

  let { filters, onChange }: { filters: RatingFilters; onChange: (f: RatingFilters) => void } = $props();

  function set<K extends keyof RatingFilters>(k: K, v: RatingFilters[K]) {
    onChange({ ...filters, [k]: v });
  }
  const num = (e: Event) => Number((e.currentTarget as HTMLSelectElement).value);
</script>

<div class="rf">
  <label class="line">
    <span>{t('ratings.my')}</span>
    <select data-testid="filter-min-my" value={filters.minMy} onchange={(e) => set('minMy', num(e))} aria-label={t('ratings.minMy')}>
      <option value={0}>{t('books.any')}</option>
      {#each [1, 2, 3, 4, 5] as n (n)}<option value={n}>≥ {n}</option>{/each}
    </select>
  </label>
  <label class="line">
    <span>{t('ratings.lib')}</span>
    <select data-testid="filter-min-lib" value={filters.minLib} onchange={(e) => set('minLib', num(e))} aria-label={t('ratings.minLib')}>
      <option value={0}>{t('books.any')}</option>
      {#each [1, 2, 3, 4, 5] as n (n)}<option value={n}>≥ {n}</option>{/each}
    </select>
  </label>
  <label class="line">
    <span>{t('ratings.ext')}</span>
    <select data-testid="filter-min-ext" value={filters.minExt} onchange={(e) => set('minExt', num(e))} aria-label={t('ratings.minExt')}>
      <option value={0}>{t('books.any')}</option>
      {#each [3, 3.5, 4, 4.5] as n (n)}<option value={n}>≥ {n}</option>{/each}
    </select>
  </label>
  <label class="line sub">
    <span>{t('ratings.minVotes')}</span>
    <select data-testid="filter-min-votes" value={filters.minExtVotes} onchange={(e) => set('minExtVotes', num(e))} aria-label={t('ratings.minVotes')}>
      <option value={0}>{t('books.any')}</option>
      {#each [5, 20, 100] as n (n)}<option value={n}>≥ {n}</option>{/each}
    </select>
  </label>
  <label class="check">
    <input type="checkbox" data-testid="filter-unrated" checked={filters.unratedByMe} onchange={(e) => set('unratedByMe', (e.currentTarget as HTMLInputElement).checked)} />
    {t('ratings.unratedByMe')}
  </label>
  <label class="line" title={t('ratings.kidsHint')}>
    <span>{t('ratings.kidsMaxAge')}</span>
    <select data-testid="filter-kids" value={filters.kidsMaxAge ?? ''} onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; set('kidsMaxAge', v === '' ? null : Number(v)); }} aria-label={t('ratings.kidsMaxAge')}>
      <option value="">{t('books.any')}</option>
      {#each KIDS_AGES as a (a)}<option value={a}>≤ {a}+</option>{/each}
    </select>
  </label>
</div>

<style>
  .rf { display: flex; flex-direction: column; gap: 2px; min-width: 240px; }
  .line span, .check { white-space: nowrap; }
  .line { display: flex; align-items: center; justify-content: space-between; gap: 10px; font-size: 13px; padding: 3px 6px; border-radius: 4px; }
  .line.sub { padding-left: 18px; color: var(--muted); }
  .line select {
    height: 26px; padding: 0 4px; border: 1px solid var(--border); border-radius: 5px; background: var(--surface);
    color: var(--ink); font: inherit; font-size: 12.5px; min-width: 76px;
  }
  .check { display: flex; align-items: center; gap: 8px; font-size: 13px; padding: 4px 6px; border-radius: 4px; }
  .check input { width: 15px; height: 15px; accent-color: var(--accent); }
</style>
