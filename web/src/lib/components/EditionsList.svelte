<script lang="ts">
  import { api, errorText } from '../api/client';
  import type { Edition } from '../api/types';
  import { t, i18nState } from '../i18n';
  import { formatDate, formatSize } from '../utils/format';
  import Icon from './Icon.svelte';

  /** All editions of the work of `bookId`, the best copy first, with what sets them apart. */
  let { lib, bookId, selectedId = null, onPick, onSend }: {
    lib: number; bookId: number; selectedId?: number | null;
    onPick: (id: number) => void; onSend: (ids: number[]) => void;
  } = $props();

  let eds = $state<Edition[] | null>(null);
  let error = $state<string | null>(null);
  $effect(() => {
    const id = bookId;
    eds = null; error = null;
    api.editions(lib, id).then((r) => (eds = r.books)).catch((e) => (error = errorText(e)));
  });
  /** A value that differs between editions is shown in bold. */
  const differs = (f: (e: Edition) => string) => !!eds && new Set(eds.map(f)).size > 1;
</script>

<div class="editions" data-testid="editions-list" role="list" aria-label={t('editions.title')}>
  {#if error}<div class="muted">{t('common.error')}: {error}</div>
  {:else if !eds}<div class="muted">{t('common.loading')}</div>
  {:else}
    {#each eds as e, i (e.id)}
      <div class="ed" role="listitem" class:selected={e.id === selectedId} class:deleted={e.deleted}>
        <button type="button" class="pick" onclick={() => onPick(e.id)} title={e.title}>
          <span class="fmt" class:hl={differs((x) => x.ext)}>{e.ext.toUpperCase()}</span>
          <span class="size" class:hl={differs((x) => formatSize(x.size))}>{formatSize(e.size)}</span>
          <span class="date" class:hl={differs((x) => x.date)}>{formatDate(e.date, i18nState.lang)}</span>
          {#if differs((x) => x.lang)}<span class="lang hl">{e.lang}</span>{/if}
          {#if e.libRating}<span class="lib" title={t('ratings.libTooltip', { n: e.libRating })}>★{e.libRating}</span>{/if}
          {#if differs((x) => x.title)}<span class="etitle" data-testid="edition-title">{e.title}</span>{/if}
          {#if e.note}<span class="note">{e.note}</span>{/if}
          {#if i === 0}<span class="best" title={t('editions.bestHint')}>{t('editions.best')}</span>{/if}
          {#if e.deleted}<span class="del">{t('books.deleted')}</span>{/if}
        </button>
        <button type="button" class="send" aria-label={t('editions.sendThis')} title={t('editions.sendThis')} onclick={() => onSend([e.id])}>
          <Icon name="send" size={14} />
        </button>
      </div>
    {/each}
  {/if}
</div>

<style>
  .editions { display: flex; flex-direction: column; background: var(--surface-alt); border-top: 1px solid var(--line-soft); padding: 4px 0; }
  .muted { color: var(--muted); font-size: 13px; padding: 6px 16px 6px 74px; }
  .ed { display: flex; align-items: center; gap: 8px; padding: 0 16px 0 74px; min-height: 34px; }
  .ed.selected { background: var(--accent-soft); }
  .ed.deleted { opacity: .6; }
  .pick { all: unset; flex: 1 1 auto; min-width: 0; display: flex; align-items: center; gap: 12px; font-size: 13px; color: var(--muted-2); cursor: pointer; }
  .pick:focus-visible { outline: 2px solid var(--focus); }
  .pick:hover .etitle { color: var(--accent); }
  .fmt { font-weight: 600; min-width: 36px; }
  .size { min-width: 60px; text-align: right; font-variant-numeric: tabular-nums; }
  .date { min-width: 84px; font-variant-numeric: tabular-nums; }
  .hl { color: var(--ink); font-weight: 600; }
  .lib { color: var(--amber); }
  .etitle { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; color: var(--ink); flex: 0 1 auto; }
  .note { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; color: var(--muted-2); flex: 0 10 auto; }
  .best { flex-shrink: 0; font-size: 11px; padding: 1px 7px; border-radius: 9px; background: var(--accent-soft); color: var(--accent-soft-ink); }
  .del { flex-shrink: 0; font-size: 11px; color: var(--danger); }
  .send { flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; width: 30px; height: 28px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); color: var(--ink); }
  .send:hover { background: var(--surface-hover); }
  @media (max-width: 900px) {
    .ed, .muted { padding-left: 12px; padding-right: 12px; }
    .date { display: none; }
  }
</style>
