<script lang="ts">
  import VirtualList from './VirtualList.svelte';
  import Icon from './Icon.svelte';
  import { normalize } from '../utils/normalize';
  import { t } from '../i18n';
  import {
    SCRIPT_LABEL, letterAt, scriptOf, scriptsByCount, stripLetters, type Script,
  } from '../utils/scripts';

  let {
    title,
    filterLabel,
    rows, // [id, name, count][]
    letters, // [letter, count, firstIndex][]
    selectedId,
    onSelect,
    width,
  }: {
    title: string;
    filterLabel: string;
    rows: [number, string, number][];
    letters: [string, number, number][];
    selectedId: number | null;
    onSelect: (id: number) => void;
    width?: number;
  } = $props();

  const ROW = 36;
  let query = $state('');
  let scrollToIndex = $state<number | null>(null);
  let activeIndex = $state(-1);
  let topIndex = $state(0);
  let script = $state<Script | null>(null);

  // Normalizing 50k names is the expensive part — do it once per `rows` change
  // (not per keystroke) so filtering stays comfortably under the 30ms budget.
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

  const scripts = $derived(scriptsByCount(letters));
  const strip = $derived(script ? stripLetters(script, letters) : []);
  // The letter under the top of the list (the strip marks it, the script tab follows it).
  // After a jump (letter, script tab) the target stays marked until the user scrolls: a group
  // near the end of the list cannot reach the top of the viewport.
  let pinnedLetter = $state<string | null>(null);
  const currentLetter = $derived(query ? null : pinnedLetter ?? letterAt(letters, topIndex)?.[0] ?? null);
  function userScrolled() { pinnedLetter = null; }
  const selectedIndex = $derived(selectedId === null ? -1 : rows.findIndex((r) => r[0] === selectedId));

  // Open at the selected name, or else at the start of the main script (А for a Russian
  // library, not at the Latin names or the "#" group); once per list and per selection.
  let positionedFor: unknown = null;
  let positionedId: number | null = null;
  $effect(() => {
    if (!rows.length || !letters.length) return;
    if (positionedFor === rows && positionedId === selectedId) return;
    const firstTime = positionedFor !== rows;
    positionedFor = rows;
    positionedId = selectedId;
    if (selectedIndex >= 0) {
      const visible = selectedIndex >= topIndex && selectedIndex < topIndex + 12;
      if (firstTime || !visible) scrollToIndex = Math.max(0, selectedIndex - 4);
      script = scriptOf(letters.length ? letterAt(letters, selectedIndex)?.[0] ?? '#' : '#');
    } else if (firstTime) {
      const main = scripts[0] ?? 'cyr';
      script = main;
      const first = letters.find((l) => scriptOf(l[0]) === main);
      scrollToIndex = first ? first[2] : 0;
      pinnedLetter = first ? first[0] : null;
    }
  });

  // The script tab follows scrolling.
  $effect(() => {
    if (currentLetter) {
      const s = scriptOf(currentLetter);
      if (s !== script) script = s;
    }
  });

  function jumpToLetter(letter: string, firstIndex: number) {
    pinnedLetter = letter;
    scrollToIndex = firstIndex;
  }

  function pickScript(s: Script) {
    script = s;
    const first = letters.find((l) => scriptOf(l[0]) === s);
    if (first) jumpToLetter(first[0], first[2]);
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
    } else if (e.key === 'Enter') {
      const i = activeIndex >= 0 ? activeIndex : 0;
      if (filtered[i]) onSelect(filtered[i][0]);
    } else if (e.key === 'Escape') {
      query = '';
    }
  }

  $effect(() => {
    // new filter text → start keyboard navigation from the top
    query;
    activeIndex = -1;
  });
</script>

<section aria-label={title} class="browser pane-list" style:--w={width ? `${width}px` : undefined}>
  <div class="head">
    <div class="head-row">
      <h2>{title}</h2>
      {#if scripts.length > 1 && !query}
        <div class="scripts" role="group" aria-label={t('browse.alphabet')}>
          {#each scripts as s (s)}
            <button type="button" class:on={s === script} aria-pressed={s === script} onclick={() => pickScript(s)}>{SCRIPT_LABEL[s]}</button>
          {/each}
        </div>
      {/if}
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
      {#if query}
        <button type="button" class="clear" aria-label={t('common.clear')} onclick={() => (query = '')}><Icon name="close" size={14} /></button>
      {/if}
    </label>
    {#if query}
      <div class="match-count">{t('browse.matches', { count: filtered.length })}</div>
    {/if}
  </div>
  <div class="body">
    <div class="list" onwheel={userScrolled} ontouchmove={userScrolled} onkeydown={userScrolled} onpointerdown={userScrolled} role="presentation">
      {#if rows.length && !filtered.length}
        <div class="nothing">{t('search.noResults')}</div>
      {/if}
      <VirtualList items={filtered} itemHeight={ROW} bind:scrollToIndex onRangeChange={(_s, _e, first) => (topIndex = first)}>
        {#snippet row(r, i)}
          <button
            type="button"
            class="arow"
            class:active={r[0] === selectedId}
            class:hover-active={i === activeIndex}
            style="height: {ROW}px"
            title={r[1]}
            onclick={() => { positionedId = r[0]; onSelect(r[0]); activeIndex = i; }}
          >
            <span class="name">{r[1]}</span>
            <span class="count">{r[2]}</span>
          </button>
        {/snippet}
      </VirtualList>
    </div>
    {#if !query && strip.length}
      <div class="letters" aria-label={t('browse.jumpToLetter')}>
        {#each strip as l, i (i)}
          <button
            type="button"
            class:active={l.letter === currentLetter}
            disabled={l.index === null}
            aria-label={l.index === null ? `${l.letter}: 0` : `${l.letter}: ${l.count}`}
            onclick={() => l.index !== null && jumpToLetter(l.letter, l.index)}
          >{l.letter}</button>
        {/each}
      </div>
    {/if}
  </div>
</section>

<style>
  .browser {
    flex: 0 1 var(--w, 280px); min-width: 200px; display: flex; flex-direction: column;
    border-right: 1px solid var(--line); background: var(--surface-alt); min-height: 0;
  }
  .head { padding: 14px 12px 8px 16px; display: flex; flex-direction: column; gap: 10px; flex-shrink: 0; }
  .head-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 26px; }
  h2 { margin: 0; font-size: 16px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .scripts { display: flex; border: 1px solid var(--border); border-radius: 6px; overflow: hidden; flex-shrink: 0; }
  .scripts button {
    height: 24px; padding: 0 8px; border: none; background: var(--surface); color: var(--muted-2);
    font-size: 12px; font-variant-numeric: tabular-nums;
  }
  .scripts button + button { border-left: 1px solid var(--border); }
  .scripts button.on { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 600; }
  .filter-box {
    display: flex; align-items: center; gap: 8px; height: 34px; padding: 0 6px 0 10px;
    border: 1px solid var(--border); border-radius: 6px; background: var(--surface);
  }
  .filter-box:focus-within { border-color: var(--accent); }
  .filter-box input {
    border: none; outline: none; flex-grow: 1; min-width: 0; font: inherit; font-size: 14px;
    background: transparent; color: var(--ink);
  }
  .clear { display: flex; border: none; background: none; color: var(--muted); padding: 4px; border-radius: 4px; }
  .clear:hover { background: var(--surface-hover); }
  .match-count { font-size: 12px; color: var(--muted); margin-top: -4px; }
  .nothing { padding: 16px; color: var(--muted); font-size: 14px; }
  .body { flex-grow: 1; display: flex; min-height: 0; }
  .list { flex-grow: 1; min-width: 0; padding: 0 2px 0 8px; position: relative; }
  .arow {
    display: flex; justify-content: space-between; align-items: center; height: 36px; width: 100%;
    padding: 0 10px; border-radius: 6px; font-size: 14px; color: var(--ink); text-decoration: none;
    border: none; background: none; text-align: left;
  }
  .arow:hover { background: var(--surface-hover); text-decoration: none; }
  .arow.active { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 600; }
  .arow.hover-active { outline: 1px dashed var(--border-dashed); outline-offset: -1px; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .count { font-size: 12px; color: var(--muted); flex-shrink: 0; margin-left: 8px; font-variant-numeric: tabular-nums; }
  .letters {
    width: 24px; flex-shrink: 0; display: flex; flex-direction: column; align-items: center;
    padding: 2px 0 8px; overflow-y: auto; scrollbar-width: none;
  }
  .letters button {
    display: flex; align-items: center; justify-content: center; width: 22px; flex: 1 1 0;
    min-height: 13px; max-height: 18px; font-size: 11px; line-height: 1;
    color: var(--muted-2); background: none; border: none; padding: 0; font-weight: 500; border-radius: 3px;
  }
  .letters button:hover:not(:disabled) { color: var(--accent); background: var(--surface-hover); }
  .letters button:disabled { color: var(--muted); opacity: .35; cursor: default; font-weight: 400; }
  .letters button.active { color: var(--accent); font-weight: 700; }
  @media (max-width: 900px) {
    .browser { flex: 1 1 auto; border-right: none; }
    .letters button { min-height: 16px; font-size: 12px; }
  }
</style>
