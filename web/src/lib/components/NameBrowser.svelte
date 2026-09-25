<script lang="ts">
  import VirtualList from './VirtualList.svelte';
  import Icon from './Icon.svelte';
  import { normalize } from '../utils/normalize';
  import { t } from '../i18n';

  let {
    title,
    filterLabel,
    rows, // [id, name, count][]
    letters, // [letter, count, firstIndex][]
    selectedId,
    onSelect,
  }: {
    title: string;
    filterLabel: string;
    rows: [number, string, number][];
    letters: [string, number, number][];
    selectedId: number | null;
    onSelect: (id: number) => void;
  } = $props();

  let query = $state('');
  let scrollToIndex = $state<number | null>(null);
  let activeIndex = $state(-1);
  let listEl: HTMLDivElement | undefined;

  // Normalizing 50k names is the expensive part — do it once per `rows` change
  // (not per keystroke) so filtering stays comfortably under the 30ms budget.
  // Plain (non-reactive) cache keyed by array identity: an $effect, not a
  // nested $derived, so it truly only reruns when `rows` itself changes.
  let normalizedRows: string[] = [];
  let normalizedFor: unknown = null;
  $effect(() => {
    normalizedRows = rows.map((r) => normalize(r[1]));
    normalizedFor = rows;
  });

  const filtered = $derived.by(() => {
    const q = normalize(query);
    if (!q) return rows;
    const norm = normalizedFor === rows ? normalizedRows : rows.map((r) => normalize(r[1]));
    const start = performance.now();
    const out: typeof rows = [];
    for (let i = 0; i < rows.length; i++) {
      if (norm[i].includes(q)) out.push(rows[i]);
    }
    if (import.meta.env.DEV) {
      const ms = performance.now() - start;
      if (ms > 30) console.warn(`[NameBrowser] filter took ${ms.toFixed(1)}ms for ${rows.length} rows`);
    }
    return out;
  });

  const activeLetter = $derived.by(() => {
    if (selectedId === null) return null;
    const row = rows.find((r) => r[0] === selectedId);
    return row ? normalize(row[1]).charAt(0).toUpperCase() : null;
  });

  function jumpToLetter(firstIndex: number) {
    scrollToIndex = firstIndex;
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      activeIndex = Math.min(filtered.length - 1, activeIndex + 1);
      scrollToIndex = activeIndex;
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      activeIndex = Math.max(0, activeIndex - 1);
      scrollToIndex = activeIndex;
    } else if (e.key === 'Enter' && activeIndex >= 0 && filtered[activeIndex]) {
      onSelect(filtered[activeIndex][0]);
    }
  }
</script>

<section aria-label={title} class="browser">
  <div class="head">
    <div class="head-row">
      <h2>{title}</h2>
      <span class="muted">{t('authors.sortedByLastName')}</span>
    </div>
    <label class="filter-box">
      <Icon name="search" size={16} class="muted-icon" />
      <input
        type="text"
        aria-label={filterLabel}
        placeholder={filterLabel}
        bind:value={query}
        onkeydown={onKeydown}
      />
    </label>
  </div>
  <div class="body">
    <div class="list" bind:this={listEl}>
      <VirtualList items={filtered} itemHeight={36} bind:scrollToIndex>
        {#snippet row(r, i)}
          <button
            type="button"
            class="arow"
            class:active={r[0] === selectedId}
            class:hover-active={i === activeIndex}
            style="height: 36px"
            onclick={() => { onSelect(r[0]); activeIndex = i; }}
          >
            <span class="name">{r[1]}</span>
            <span class="count">{r[2]}</span>
          </button>
        {/snippet}
      </VirtualList>
    </div>
    <div class="letters" aria-label="Jump to letter">
      {#each letters as l (l[0])}
        <button
          type="button"
          class:active={l[0] === activeLetter}
          onclick={() => jumpToLetter(l[2])}
        >{l[0]}</button>
      {/each}
    </div>
  </div>
</section>

<style>
  .browser {
    width: 280px; flex-shrink: 0; display: flex; flex-direction: column;
    border-right: 1px solid var(--line); background: var(--surface-alt); min-height: 0;
  }
  .head { padding: 16px 16px 10px; display: flex; flex-direction: column; gap: 10px; flex-shrink: 0; }
  .head-row { display: flex; align-items: baseline; justify-content: space-between; }
  h2 { margin: 0; font-size: 16px; font-weight: 600; }
  .muted { font-size: 12px; color: var(--muted); }
  .filter-box {
    display: flex; align-items: center; gap: 8px; height: 34px; padding: 0 10px;
    border: 1px solid var(--border); border-radius: 6px; background: var(--surface);
  }
  .filter-box input {
    border: none; outline: none; flex-grow: 1; font: inherit; font-size: 14px;
    background: transparent; color: var(--ink);
  }
  .body { flex-grow: 1; display: flex; min-height: 0; }
  .list { flex-grow: 1; min-width: 0; padding: 0 4px 0 8px; }
  .arow {
    display: flex; justify-content: space-between; align-items: center; height: 36px; width: 100%;
    padding: 0 12px; border-radius: 6px; font-size: 14px; color: var(--ink); text-decoration: none;
    border: none; background: none; text-align: left;
  }
  .arow:hover { background: var(--surface-hover); text-decoration: none; }
  .arow.active { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 600; }
  .arow.hover-active { outline: 1px dashed var(--border-dashed); outline-offset: -1px; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { font-size: 12px; color: var(--muted); flex-shrink: 0; margin-left: 8px; }
  .letters {
    width: 26px; flex-shrink: 0; display: flex; flex-direction: column; align-items: center;
    padding: 6px 0; overflow-y: auto;
  }
  .letters button {
    display: block; width: 22px; text-align: center; font-size: 11px; line-height: 17px;
    color: var(--muted); background: none; border: none; padding: 0; font-weight: 400;
  }
  .letters button:hover { color: var(--accent); }
  .letters button.active { color: var(--accent); font-weight: 700; }
</style>
