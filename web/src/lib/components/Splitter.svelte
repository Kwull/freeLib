<script lang="ts">
  // A vertical drag handle that resizes the element next to it: pointer drag (mouse, touch, pen;
  // the pointer is captured, so the drag keeps working over other panes and iframes and ends
  // even when released outside the window), arrow keys (Shift = bigger steps), Home/End =
  // min/max, double click or Enter = default width.
  // `side` says where the resized element is: 'before' (to the left, dragging right makes it
  // wider) or 'after' (to the right, dragging left makes it wider). With `target = 'sibling'`
  // (panes) the neighbour on `side` is measured: a drag starts from its width on screen, and
  // the committed width is what the layout actually gave, so a pane squeezed by a narrow
  // window has no dead zone at the start of the next drag. `'none'` (table columns, whose
  // grid tracks are exactly `value`) uses `value`.
  import { onDestroy } from 'svelte';

  let {
    value,
    min,
    max,
    label,
    side = 'before',
    target = 'sibling',
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
    target?: 'sibling' | 'none';
    onInput: (v: number) => void;
    onCommit: (v: number) => void;
    onReset: () => void;
    class?: string;
  } = $props();

  let el: HTMLDivElement | undefined = $state();
  let dragging = $state(false);
  let pointerId = -1;
  let startX = 0;
  let startV = 0;
  let last = 0;

  const clamp = (v: number) => Math.round(Math.min(max, Math.max(min, v)));
  const dir = $derived(side === 'before' ? 1 : -1);

  function targetEl(): HTMLElement | null {
    if (!el || target === 'none') return null;
    return (side === 'before' ? el.previousElementSibling : el.nextElementSibling) as HTMLElement | null;
  }
  /** The rendered width of the resized element (falls back to `value`). */
  function actual(): number {
    const w = targetEl()?.getBoundingClientRect().width;
    return w && Number.isFinite(w) ? w : value;
  }

  function down(e: PointerEvent) {
    // primary button / touch contact / pen tip only
    if (e.button !== 0 || !e.isPrimary || dragging) return;
    e.preventDefault();
    e.stopPropagation();
    const h = e.currentTarget as HTMLElement;
    try { h.setPointerCapture(e.pointerId); } catch { /* pointer already gone */ }
    h.focus({ preventScroll: true });
    pointerId = e.pointerId;
    dragging = true;
    startX = e.clientX;
    startV = clamp(actual());
    last = startV;
    document.body.classList.add('col-resizing');
  }
  function move(e: PointerEvent) {
    if (!dragging || e.pointerId !== pointerId) return;
    const v = clamp(startV + dir * (e.clientX - startX));
    if (v === last) return;
    last = v;
    onInput(v);
  }
  function finish(commit: boolean) {
    if (!dragging) return;
    dragging = false;
    document.body.classList.remove('col-resizing');
    if (el && pointerId >= 0 && el.hasPointerCapture?.(pointerId)) {
      try { el.releasePointerCapture(pointerId); } catch { /* already released */ }
    }
    pointerId = -1;
    if (!commit || last === startV) return;
    const wanted = last;
    // commit what the layout gave, once it has been laid out
    requestAnimationFrame(() => {
      const got = clamp(actual());
      onCommit(Math.abs(got - wanted) > 2 && got < wanted ? got : wanted);
    });
  }
  function up(e: PointerEvent) {
    if (e.pointerId === pointerId) finish(true);
  }
  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? 64 : 16;
    let v: number | null = null;
    const cur = clamp(value);
    if (e.key === 'ArrowLeft') v = cur - dir * step;
    else if (e.key === 'ArrowRight') v = cur + dir * step;
    else if (e.key === 'Home') v = min;
    else if (e.key === 'End') v = max;
    else if (e.key === 'Enter') {
      e.preventDefault();
      onReset();
      return;
    } else if (e.key === 'Escape' && dragging) {
      // cancel a drag: back to where it started
      e.preventDefault();
      onInput(startV);
      last = startV;
      finish(false);
      onCommit(startV);
      return;
    }
    if (v === null) return;
    e.preventDefault();
    e.stopPropagation();
    v = clamp(v);
    onInput(v);
    onCommit(v);
  }

  onDestroy(() => {
    if (dragging) document.body.classList.remove('col-resizing');
  });
</script>

<!-- A focusable separator is a widget (ARIA "window splitter" pattern). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  bind:this={el}
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
  onlostpointercapture={() => finish(true)}
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
