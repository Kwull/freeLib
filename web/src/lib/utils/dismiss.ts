// One mechanism for every dropdown, menu and popover: `use:dismissable={{ onClose, trigger }}`
// on the open popup element. It closes the popup
//  - on a pointer press outside it (and outside its trigger, which toggles it itself),
//  - on Escape (the most recently opened popup only), returning focus to the trigger,
//  - when another popup opens (unless this one contains it: a nested popover),
//  - on navigation (in-app `navigate()`, back/forward).
// Closing on choosing an item stays with the menu (multi-toggle menus such as Columns stay open
// while toggling inside). `enabled: false` makes it inert (a panel that is only a popup on phones).

export type DismissOptions = {
  onClose: () => void;
  /** the button that opens the popup: presses on it are not "outside", Escape focuses it */
  trigger?: HTMLElement | null | (() => HTMLElement | null | undefined);
  /** default true */
  enabled?: boolean;
};

type Entry = { node: HTMLElement; opts: DismissOptions };

/** Open popups, oldest first. */
const open: Entry[] = [];

export const NAVIGATE_EVENT = 'freelib:navigate';

function triggerOf(o: DismissOptions): HTMLElement | null {
  const t = typeof o.trigger === 'function' ? o.trigger() : o.trigger;
  return t ?? null;
}

function close(e: Entry) {
  const i = open.indexOf(e);
  if (i >= 0) open.splice(i, 1);
  e.opts.onClose();
}

function onPointerDown(ev: PointerEvent) {
  const target = ev.target as Node | null;
  if (!target) return;
  // innermost first, so that closing a parent does not run twice
  for (const e of open.slice().reverse()) {
    if (e.node.contains(target)) continue;
    const t = triggerOf(e.opts);
    if (t && t.contains(target)) continue;
    // a press inside a popup nested in this one is inside
    if (open.some((o) => o !== e && e.node.contains(o.node) && o.node.contains(target))) continue;
    close(e);
  }
}

function onKeyDown(ev: KeyboardEvent) {
  if (ev.key !== 'Escape' || ev.defaultPrevented || !open.length) return;
  const e = open[open.length - 1];
  ev.preventDefault();
  ev.stopPropagation();
  const t = triggerOf(e.opts);
  close(e);
  if (t && document.contains(t)) t.focus();
}

function onNavigate() {
  for (const e of open.slice().reverse()) close(e);
}

let listening = false;
function listen() {
  if (listening) return;
  listening = true;
  document.addEventListener('pointerdown', onPointerDown, true);
  // capture: before the page's own Escape handlers (clearing a search box, closing a dialog)
  document.addEventListener('keydown', onKeyDown, true);
  window.addEventListener(NAVIGATE_EVENT, onNavigate);
  window.addEventListener('popstate', onNavigate);
}

function register(e: Entry) {
  listen();
  // opening a popup closes the others, except the ones it is nested in
  for (const o of open.slice().reverse()) if (!o.node.contains(e.node)) close(o);
  open.push(e);
}

function unregister(e: Entry) {
  const i = open.indexOf(e);
  if (i >= 0) open.splice(i, 1);
}

export function dismissable(node: HTMLElement, opts: DismissOptions) {
  const entry: Entry = { node, opts };
  if (opts.enabled !== false) register(entry);
  return {
    update(next: DismissOptions) {
      const was = entry.opts.enabled !== false;
      entry.opts = next;
      const now = next.enabled !== false;
      if (now && !was) register(entry);
      else if (!now && was) unregister(entry);
    },
    destroy() {
      unregister(entry);
    },
  };
}

/** Number of open popups (tests / debugging). */
export function openPopups(): number {
  return open.length;
}
