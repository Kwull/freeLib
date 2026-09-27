<script lang="ts">
  import { tick, untrack } from 'svelte';
  import VirtualList from './VirtualList.svelte';
  import Icon from './Icon.svelte';
  import { normalize } from '../utils/normalize';
  import { latinKey, nameRank, wordsOf } from '../utils/phonetic';
  import { t } from '../i18n';
  import { nameFilterState, getNameFilter, setNameFilter } from '../stores/nameFilter.svelte';
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
    loading = false,
    progressText = null,
    progress = null,
    filterKey = null,
  }: {
    title: string;
    filterLabel: string;
    rows: [number, string, number][];
    letters: [string, number, number][];
    selectedId: number | null;
    onSelect: (id: number) => void;
    width?: number;
    /** The list is still downloading: skeleton rows (and how far it got). */
    loading?: boolean;
    progressText?: string | null;
    progress?: number | null;
    /** keeps the filter text (nameFilter store): across visits, and typed while loading */
    filterKey?: string | null;
  } = $props();

  const ROW = 36;
  // the filter text is never reset by a load: it starts from the store and is written back
  let query = $state(untrack(() => (filterKey ? getNameFilter(filterKey) : '')));
  // (Authors ↔ Series reuse this component: each list has its own filter)
  let queryKey = untrack(() => filterKey);
  $effect.pre(() => {
    const k = filterKey;
    untrack(() => {
      if (k === queryKey) return;
      queryKey = k;
      query = k ? getNameFilter(k) : '';
    });
  });
  $effect(() => { const q = query; if (queryKey) setNameFilter(queryKey, q); });
  let filterInput = $state<HTMLInputElement | undefined>();
  // typed into the placeholder box while the page loaded: the focus moves on to this box
  $effect(() => {
    const el = filterInput;
    if (!el || !filterKey || nameFilterState.focusKey !== filterKey) return;
    nameFilterState.focusKey = null;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  });
  let scrollToIndex = $state<number | null>(null);
  let activeIndex = $state(-1);
  let listRef = $state<{ reveal: (i: number) => void } | undefined>();
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

  // The phonetic keys of each name's words that differ from the word (` asimov aisek`), so a
  // Latin spelling finds a Cyrillic name (`asimov`, `azimoff` → Азимов). Computed in slices
  // after the list arrives (memoized per word: first names and patronymics repeat); until then
  // the filter matches the text only.
  let hayRows: string[] = [];
  let hayFor: unknown = null;
  let keysVersion = $state(0);
  $effect(() => {
    const r = rows;
    const out: string[] = new Array(r.length);
    const memo = new Map<string, string | null>();
    const key = (w: string) => {
      let k = memo.get(w);
      if (k === undefined) {
        k = latinKey(w);
        if (k === w) k = null;
        memo.set(w, k);
      }
      return k;
    };
    let i = 0;
    let cancelled = false;
    const step = () => {
      if (cancelled) return;
      const norm = normalizedFor === r ? normalizedRows : null;
      const end = Math.min(r.length, i + 5000);
      for (; i < end; i++) {
        const n = norm?.[i] ?? normalize(r[i][1]);
        let h = '';
        for (const w of wordsOf(n)) {
          const k = key(w);
          if (k) h += ` ${k}`;
        }
        out[i] = h;
      }
      if (i < r.length) setTimeout(step, 0);
      else {
        hayRows = out;
        hayFor = r;
        keysVersion++;
      }
    };
    const timer = setTimeout(step, 0);
    return () => { cancelled = true; clearTimeout(timer); };
  });

  /** Rows matching the filter, best first (like the server's search): the last name is the
   *  query word, then starts with it, then another word is / starts with it, then an infix
   *  match; more books first among equals. Every query word must match the start of a word
   *  (as typed or by its phonetic key), or the whole query must occur in the name. */
  const filtered = $derived.by(() => {
    keysVersion;
    const q = normalize(query);
    if (!q) return rows;
    const norm = normalizedFor === rows ? normalizedRows : rows.map((r) => normalize(r[1]));
    const hay = hayFor === rows ? hayRows : null;
    const start = performance.now();
    const tokens = wordsOf(q);
    const needles = tokens.map((t) => {
      const k = [...t].length >= 3 ? latinKey(t) : null;
      return { t: ` ${t}`, k: hay && k ? ` ${k}` : null };
    });
    const hits: { i: number; words: boolean }[] = [];
    const one = needles.length === 1 ? needles[0] : null;
    for (let i = 0; i < rows.length; i++) {
      const n = norm[i];
      const infix = n.includes(q);
      let words: boolean;
      if (one) {
        // one word: a word start implies an infix match, so most rows cost one or two scans
        words = infix ? n.startsWith(q) || n.includes(one.t) : !!(one.k && hay && hay[i].includes(one.k));
      } else {
        words = needles.length > 0;
        for (const nd of needles) {
          if (!(n.startsWith(nd.t.slice(1)) || n.includes(nd.t)) && !(nd.k && hay && hay[i].includes(nd.k))) { words = false; break; }
        }
      }
      if (words || infix) hits.push({ i, words });
    }
    // ranking every hit of a one-letter filter is not worth it: those stay in list order
    let out: typeof rows;
    if (hits.length <= 20000) {
      const ranked = hits.map((h) => ({ ...h, rank: h.words ? nameRank(norm[h.i], tokens) : 4 }));
      ranked.sort((a, b) => a.rank - b.rank || rows[b.i][2] - rows[a.i][2] || a.i - b.i);
      out = ranked.map((h) => rows[h.i]);
    } else {
      out = hits.map((h) => rows[h.i]);
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
    // a filtered list (a filter kept from the last visit, or typed while loading) stays at
    // its best matches: row indexes of the whole list mean nothing there
    if (untrack(() => query)) return;
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

  // Keyboard on the list itself (a listbox): ↑/↓, Home/End, PgUp/PgDn move the focus between
  // the names (the virtual list renders the new one first), Enter opens it.
  let listEl: HTMLDivElement | undefined = $state();
  function focusRow(i: number) {
    if (!filtered.length) return;
    i = Math.max(0, Math.min(filtered.length - 1, i));
    activeIndex = i;
    listRef?.reveal(i);
    tick().then(() => listEl?.querySelector<HTMLElement>(`[data-i="${i}"]`)?.focus());
  }
  function onListKeydown(e: KeyboardEvent) {
    const row = (e.target as HTMLElement).closest<HTMLElement>('[data-i]');
    if (!row || e.altKey || e.ctrlKey || e.metaKey) return;
    const i = Number(row.dataset.i);
    const page = Math.max(1, Math.floor((listEl?.clientHeight ?? 360) / ROW) - 1);
    const to = e.key === 'ArrowDown' ? i + 1 : e.key === 'ArrowUp' ? i - 1 : e.key === 'Home' ? 0
      : e.key === 'End' ? filtered.length - 1 : e.key === 'PageDown' ? i + page : e.key === 'PageUp' ? i - page : null;
    if (to === null) return;
    e.preventDefault();
    focusRow(to);
  }

  // Letter strip: tap a letter, or (phones) press and slide along the strip — the letter
  // under the finger is shown large in a bubble and the list follows it.
  let scrub = $state<{ letter: string; y: number } | null>(null);
  let stripEl: HTMLDivElement | undefined = $state();
  function letterFromPoint(x: number, y: number): HTMLButtonElement | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLButtonElement>('.letters button');
    return el && stripEl?.contains(el) ? el : null;
  }
  function scrubAt(e: PointerEvent) {
    if (!stripEl) return;
    const r = stripEl.getBoundingClientRect();
    // clamp to the strip so a finger sliding past its end still picks the first / last letter
    const y = Math.max(r.top + 2, Math.min(r.bottom - 2, e.clientY));
    const btn = letterFromPoint(r.left + r.width / 2, y);
    if (!btn || btn.disabled) return;
    const i = Number(btn.dataset.strip);
    const l = strip[i];
    if (!l || l.index === null) return;
    scrub = { letter: l.letter, y: y - r.top };
    if (pinnedLetter !== l.letter) jumpToLetter(l.letter, l.index);
  }
  function onStripDown(e: PointerEvent) {
    if (e.pointerType === 'mouse') return; // mice click the buttons
    try { stripEl?.setPointerCapture(e.pointerId); } catch { /* not an active pointer */ }
    e.preventDefault();
    scrubAt(e);
  }
  function onStripMove(e: PointerEvent) {
    if (scrub) scrubAt(e);
  }
  function onStripUp() {
    scrub = null;
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
        bind:this={filterInput}
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
    <div class="list" bind:this={listEl} onwheel={userScrolled} ontouchmove={userScrolled} onkeydown={(e) => { userScrolled(); onListKeydown(e); }} onpointerdown={userScrolled} role="presentation">
      {#if loading && !rows.length}
        <div class="loading" aria-busy="true">
          {#if progressText}
            <div class="load-progress">
              <div class="load-bar"><div style:width="{(progress ?? 0) * 100}%"></div></div>
              <span>{progressText}</span>
            </div>
          {/if}
          {#each [62, 48, 71, 55, 66, 43, 58, 69, 51, 64, 46, 60] as w, i (i)}
            <div class="sk-row" aria-hidden="true"><span class="sk" style:width="{w}%"></span><span class="sk n"></span></div>
          {/each}
        </div>
      {/if}
      {#if rows.length && !filtered.length}
        <div class="nothing">{t('search.noResults')}</div>
      {/if}
      <VirtualList bind:this={listRef} items={filtered} itemHeight={ROW} bind:scrollToIndex onRangeChange={(_s, _e, first) => (topIndex = first)}>
        {#snippet row(r, i)}
          <button
            type="button"
            data-i={i}
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
      <div class="letters" class:scrubbing={!!scrub} aria-label={t('browse.jumpToLetter')} bind:this={stripEl} data-testid="letter-strip"
        role="group" onpointerdown={onStripDown} onpointermove={onStripMove} onpointerup={onStripUp} onpointercancel={onStripUp}>
        {#each strip as l, i (i)}
          <button
            type="button"
            data-strip={i}
            class:active={l.letter === currentLetter}
            disabled={l.index === null}
            aria-label={l.index === null ? `${l.letter}: 0` : `${l.letter}: ${l.count}`}
            onclick={() => l.index !== null && jumpToLetter(l.letter, l.index)}
          >{l.letter}</button>
        {/each}
      </div>
      {#if scrub}
        <div class="bubble" style:top="{scrub.y}px" aria-hidden="true" data-testid="letter-bubble">{scrub.letter}</div>
      {/if}
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
  .loading { position: absolute; inset: 0; padding: 0 2px 0 0; overflow: hidden; }
  .sk-row { display: flex; align-items: center; justify-content: space-between; height: 36px; padding: 0 10px; gap: 12px; }
  .sk { display: block; height: 10px; border-radius: 5px; background: var(--surface-hover); animation: pulse 1.2s ease-in-out infinite; }
  .sk.n { width: 22px; flex-shrink: 0; }
  .load-progress { display: flex; flex-direction: column; gap: 4px; padding: 4px 10px 8px; font-size: 12px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .load-bar { height: 4px; border-radius: 2px; background: var(--surface-hover); overflow: hidden; }
  .load-bar div { height: 100%; background: var(--accent); transition: width .3s; }
  @keyframes pulse { 50% { opacity: .45; } }
  .nothing { padding: 16px; color: var(--muted); font-size: 14px; }
  .body { flex-grow: 1; display: flex; min-height: 0; position: relative; }
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
  .arow:focus-visible { outline: 2px solid var(--focus); outline-offset: -2px; }
  .bubble {
    position: absolute; right: 52px; width: 56px; height: 56px; margin-top: -28px; border-radius: 28px;
    display: flex; align-items: center; justify-content: center; pointer-events: none; z-index: 3;
    background: var(--accent); color: #fff; font-size: 26px; font-weight: 600; box-shadow: 0 6px 20px rgba(0,0,0,.25);
  }
  @media (max-width: 900px) {
    .browser { flex: 1 1 auto; border-right: none; }
    /* phones: a wider strip for the thumb (the whole strip is a scrub area, see onStripDown);
       letters as tall as the strip allows, at least 24px when there is room */
    .letters { width: 36px; touch-action: none; padding: 4px 0 8px; }
    .letters button { width: 36px; min-height: 16px; max-height: 28px; font-size: 13px; }
    .letters.scrubbing button.active { background: var(--accent-soft); }
    /* overlay scrollbars sit on the right edge of the list: keep the counts clear of them */
    .arow { padding-right: 16px; }
    .list { padding-right: 4px; }
  }
</style>
