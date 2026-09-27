// Positioned popups that are never clipped: `use:popover={{ anchor, placement }}` on a menu or
// popover element (usually together with `use:dismissable`).
//
// The element stays where it is in the DOM (tab order, scoped styles and `dismissable` nesting keep
// working) but is shown in the browser's top layer (the Popover API, `popover="manual"`), so no
// ancestor's `overflow`, `transform` or stacking context can clip or cover it. It is placed with
// fixed coordinates next to its anchor:
//  - below the anchor, or above it when there is more room there (flip),
//  - aligned to the anchor's start or end edge, then shifted to stay inside the viewport (shift),
//  - capped to the room left (max-height / max-width, scrolling inside),
// and follows the anchor on scroll, resize and when either element changes size.
// Browsers without the Popover API get the same fixed positioning with a high z-index.

export type Placement = 'bottom-start' | 'bottom-end' | 'top-start' | 'top-end';

export type PopoverOptions = {
  /** the element the popup is placed next to (usually its trigger button) */
  anchor: HTMLElement | null | undefined | (() => HTMLElement | null | undefined);
  /** preferred side and alignment; default 'bottom-start' */
  placement?: Placement;
  /** gap between anchor and popup, px; default 4 */
  offset?: number;
  /** minimal distance from the viewport edges, px; default 8 */
  margin?: number;
  /** false: leave the element alone (e.g. a panel that is a popup only on phones) */
  enabled?: boolean;
};

function anchorOf(o: PopoverOptions): HTMLElement | null {
  const a = typeof o.anchor === 'function' ? o.anchor() : o.anchor;
  return a ?? null;
}

const supportsPopover = typeof HTMLElement !== 'undefined' && 'showPopover' in HTMLElement.prototype;

/** Places `node` next to `anchor` inside the viewport. Exported for tests and one-off use. */
export function place(node: HTMLElement, anchor: HTMLElement, opts: PopoverOptions = { anchor }) {
  const placement = opts.placement ?? 'bottom-start';
  const offset = opts.offset ?? 4;
  const margin = opts.margin ?? 8;
  const vv = window.visualViewport;
  const vw = document.documentElement.clientWidth || window.innerWidth;
  const vh = vv ? Math.min(window.innerHeight, vv.height) : window.innerHeight;
  const a = anchor.getBoundingClientRect();

  // measure unconstrained
  const s = node.style;
  s.maxHeight = '';
  s.maxWidth = `${Math.max(0, vw - 2 * margin)}px`;
  const r = node.getBoundingClientRect();
  let w = r.width;
  let h = r.height;

  const below = vh - a.bottom - offset - margin;
  const above = a.top - offset - margin;
  const wantBelow = placement.startsWith('bottom');
  let onBelow: boolean;
  if (wantBelow) onBelow = h <= below || below >= above;
  else onBelow = !(h <= above || above >= below);
  const room = Math.max(80, onBelow ? below : above);
  if (h > room) {
    s.maxHeight = `${room}px`;
    s.overflowY = 'auto';
    h = room;
  }
  let top = onBelow ? a.bottom + offset : a.top - offset - h;
  top = Math.max(margin, Math.min(top, vh - margin - h));

  let left = placement.endsWith('end') ? a.right - w : a.left;
  w = Math.min(w, vw - 2 * margin);
  left = Math.max(margin, Math.min(left, vw - margin - w));

  s.top = `${Math.round(top)}px`;
  s.left = `${Math.round(left)}px`;
  node.dataset.side = onBelow ? 'bottom' : 'top';
}

export function popover(node: HTMLElement, initial: PopoverOptions) {
  let opts = initial;
  let frame = 0;
  let shown = false;
  let ro: ResizeObserver | null = null;
  let observedAnchor: HTMLElement | null = null;

  const update = () => {
    frame = 0;
    const a = anchorOf(opts);
    if (!shown || !a || !node.isConnected) return;
    place(node, a, opts);
  };
  const schedule = () => { if (!frame) frame = requestAnimationFrame(update); };

  function show() {
    if (shown) return;
    shown = true;
    node.dataset.flPopover = '';
    const s = node.style;
    s.position = 'fixed';
    s.right = 'auto';
    s.bottom = 'auto';
    s.margin = '0';
    s.transform = 'none';
    s.zIndex = '1000';
    s.boxSizing = 'border-box';
    if (supportsPopover) {
      node.setAttribute('popover', 'manual');
      try { node.showPopover(); } catch { /* already shown / detached */ }
    }
    update();
    window.addEventListener('resize', schedule);
    window.visualViewport?.addEventListener('resize', schedule);
    document.addEventListener('scroll', schedule, true);
    if (typeof ResizeObserver !== 'undefined') {
      ro = new ResizeObserver(schedule);
      ro.observe(node);
      observedAnchor = anchorOf(opts);
      if (observedAnchor) ro.observe(observedAnchor);
    }
  }

  function hide() {
    if (!shown) return;
    shown = false;
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    window.removeEventListener('resize', schedule);
    window.visualViewport?.removeEventListener('resize', schedule);
    document.removeEventListener('scroll', schedule, true);
    ro?.disconnect();
    ro = null;
    if (supportsPopover && node.hasAttribute('popover')) {
      try { node.hidePopover(); } catch { /* not shown */ }
      node.removeAttribute('popover');
    }
    delete node.dataset.flPopover;
    for (const p of ['position', 'right', 'bottom', 'top', 'left', 'margin', 'transform', 'zIndex', 'boxSizing', 'maxHeight', 'maxWidth', 'overflowY'] as const) {
      node.style[p] = '';
    }
  }

  if (opts.enabled !== false) show();
  return {
    update(next: PopoverOptions) {
      opts = next;
      if (next.enabled === false) hide();
      else { show(); schedule(); }
    },
    destroy: hide,
  };
}
