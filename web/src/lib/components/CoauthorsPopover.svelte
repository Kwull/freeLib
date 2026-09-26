<script lang="ts">
  // "+N more" co-authors of a prolific author: the full list (thousands of names for authors
  // who appear in many anthologies), loaded on open, searchable, real co-authors first.
  import { api, errorText } from '../api/client';
  import VirtualList from './VirtualList.svelte';
  import Icon from './Icon.svelte';
  import { normalize } from '../utils/normalize';
  import { navigate } from '../router.svelte';
  import { t, tn } from '../i18n';

  import { dismissable } from '../utils/dismiss';

  let { lib, authorId, onClose, trigger }: { lib: number; authorId: number; onClose: () => void; trigger?: HTMLElement } = $props();

  let rows = $state<[number, string, number, number][] | null>(null);
  let error = $state<string | null>(null);
  let query = $state('');
  let input: HTMLInputElement | undefined = $state();

  $effect(() => {
    const id = authorId;
    rows = null;
    api.coauthors(lib, id).then((r) => (rows = r.rows)).catch((e) => (error = errorText(e)));
  });
  $effect(() => { input?.focus(); });

  let norm: string[] = [];
  let normFor: unknown = null;
  const filtered = $derived.by(() => {
    if (!rows) return [];
    const q = normalize(query);
    if (!q) return rows;
    if (normFor !== rows) { norm = rows.map((r) => normalize(r[1])); normFor = rows; }
    return rows.filter((_, i) => norm[i].includes(q));
  });

  function go(id: number) {
    onClose();
    navigate(`/l/${lib}/authors/${id}`);
  }
</script>

<div class="pop" role="dialog" tabindex="-1" aria-label={t('authors.coauthorsTitle')}
  use:dismissable={{ onClose, trigger: () => trigger }}>
  <div class="top">
    <label class="box">
      <Icon name="search" size={14} />
      <input bind:this={input} type="text" bind:value={query} placeholder={t('authors.filterCoauthors')} aria-label={t('authors.filterCoauthors')}
        onkeydown={(e) => { if (e.key === 'Enter' && filtered[0]) go(filtered[0][0]); }} />
    </label>
    <button type="button" class="x" aria-label={t('common.close')} onclick={onClose}><Icon name="close" size={14} /></button>
  </div>
  {#if error}
    <p class="note">{error}</p>
  {:else if !rows}
    <p class="note">{t('common.loading')}</p>
  {:else}
    <p class="note">{t('authors.coauthorsNote', { count: rows.length })}</p>
    <div class="list">
      <VirtualList items={filtered} itemHeight={34}>
        {#snippet row(r)}
          <a class="r" href="/l/{lib}/authors/{r[0]}" data-link onclick={(e) => { e.preventDefault(); go(r[0]); }}>
            <span class="n">{r[1]}</span>
            {#if r[3] > 0}<span class="tag">{t('authors.coauthor')}</span>{/if}
            <span class="c" title={tn('authors.sharedBooks', r[2])}>{r[2]}</span>
          </a>
        {/snippet}
      </VirtualList>
    </div>
  {/if}
</div>

<style>
  .pop {
    position: absolute; top: 22px; left: 0; z-index: 40; width: 380px; max-width: calc(100vw - 32px);
    background: var(--surface); border: 1px solid var(--line); border-radius: 10px;
    box-shadow: 0 12px 32px rgba(0,0,0,.18); padding: 10px; display: flex; flex-direction: column; gap: 6px;
    color: var(--ink);
  }
  .top { display: flex; gap: 6px; align-items: center; }
  .box { flex-grow: 1; display: flex; align-items: center; gap: 6px; height: 32px; padding: 0 8px; border: 1px solid var(--border); border-radius: 6px; color: var(--muted); }
  .box:focus-within { border-color: var(--accent); }
  .box input { border: none; outline: none; background: transparent; flex-grow: 1; min-width: 0; font: inherit; font-size: 13px; color: var(--ink); }
  .x { display: flex; border: none; background: none; color: var(--muted); padding: 6px; border-radius: 6px; }
  .x:hover { background: var(--surface-hover); }
  .note { margin: 0; font-size: 12px; color: var(--muted); padding: 0 2px; }
  .list { height: 320px; }
  .r { display: flex; align-items: center; gap: 8px; height: 34px; padding: 0 8px; border-radius: 6px; color: var(--ink); text-decoration: none; font-size: 13px; }
  .r:hover { background: var(--surface-hover); text-decoration: none; }
  .n { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; flex: 0 1 auto; }
  .tag { font-size: 11px; color: var(--accent-soft-ink); background: var(--accent-soft); border-radius: 4px; padding: 1px 6px; flex-shrink: 0; }
  .c { margin-left: auto; color: var(--muted); font-size: 12px; font-variant-numeric: tabular-nums; flex-shrink: 0; }
</style>
