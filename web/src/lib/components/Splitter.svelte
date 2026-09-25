<script lang="ts">
  // A vertical drag handle that resizes the element next to it: pointer drag, arrow keys
  // (Shift = bigger steps), Home/End = min/max, double click or Enter = default width.
  // `side` says where the resized element is: 'before' (to the left, dragging right makes it
  // wider) or 'after' (to the right, dragging left makes it wider).
  let {
    value,
    min,
    max,
    label,
    side = 'before',
    onInput,
    onCommit,
    onReset,
    class: className = '',
  }: {
    value: number;
    min: number;
    max: number;
    label: string;
    side?: 'before' | 'after';
    onInput: (v: number) => void;
    onCommit: (v: number) => void;
    onReset: () => void;
    class?: string;
  } = $props();

  let dragging = $state(false);
  let startX = 0;
  let startV = 0;
  let last = 0;

  const clamp = (v: number) => Math.round(Math.min(max, Math.max(min, v)));
  const dir = $derived(side === 'before' ? 1 : -1);

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    (e.currentTarget as HTMLElement).focus();
    dragging = true;
    startX = e.clientX;
    startV = value;
    last = value;
    document.body.classList.add('col-resizing');
  }
  function move(e: PointerEvent) {
    if (!dragging) return;
    last = clamp(startV + dir * (e.clientX - startX));
    onInput(last);
  }
  function up(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    document.body.classList.remove('col-resizing');
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    if (last !== startV) onCommit(last);
  }
  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? 64 : 16;
    let v: number | null = null;
    if (e.key === 'ArrowLeft') v = value - dir * step;
    else if (e.key === 'ArrowRight') v = value + dir * step;
    else if (e.key === 'Home') v = min;
    else if (e.key === 'End') v = max;
    else if (e.key === 'Enter') {
      e.preventDefault();
      onReset();
      return;
    }
    if (v === null) return;
    e.preventDefault();
    e.stopPropagation();
    v = clamp(v);
    onInput(v);
    onCommit(v);
  }
</script>

<!-- A focusable separator is a widget (ARIA "window splitter" pattern). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="splitter {className}"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label={label}
  aria-valuenow={value}
  aria-valuemin={min}
  aria-valuemax={max}
  tabindex="0"
  title={label}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  ondblclick={(e) => { e.preventDefault(); e.stopPropagation(); onReset(); }}
  onkeydown={key}
  onclick={(e) => e.stopPropagation()}
></div>

<style>
  /* 9px wide hit area centred on the border line (negative margins: takes no layout space),
     a 2px accent line on hover/focus/drag */
  .splitter {
    position: relative; flex: 0 0 9px; width: 9px; margin: 0 -5px 0 -4px; align-self: stretch; z-index: 6;
    outline: none; touch-action: none; cursor: col-resize;
  }
  .splitter::after {
    content: ''; position: absolute; top: 0; bottom: 0; left: 3.5px; width: 2px;
    background: var(--accent); opacity: 0; transition: opacity .12s;
    pointer-events: none;
  }
  .splitter:hover::after { opacity: .45; }
  .splitter:focus-visible::after, .splitter.dragging::after { opacity: 1; }
  :global(body.col-resizing), :global(body.col-resizing *) { cursor: col-resize !important; user-select: none !important; }
  @media (max-width: 900px) {
    .splitter { display: none; }
  }
</style>
