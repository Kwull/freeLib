<script lang="ts">
  // Age suitability estimate ("6+", "12+" …), a heuristic from genres and keywords.
  import { t } from '../i18n';
  import { kidsLabel } from '../utils/ratings';

  let { age }: { age: number | null | undefined } = $props();
</script>

{#if age !== null && age !== undefined}
  <span class="kids" class:adult={age >= 16} data-testid="kids-badge" title={t('ratings.kidsTooltip', { age: kidsLabel(age) })}>{kidsLabel(age)}</span>
{/if}

<style>
  .kids {
    flex-shrink: 0; font-size: 10.5px; font-weight: 600; line-height: 1; padding: 3px 5px; border-radius: 4px; white-space: nowrap;
    color: var(--kids-ink, #2f6b3a); background: var(--kids-bg, #e3f1e5); font-variant-numeric: tabular-nums;
  }
  .kids.adult { color: var(--muted-2); background: var(--surface-hover); }
</style>
