// Resizable layout: widths of the side panes and of the book table columns, the collapsed
// state of the details pane. Backed by the user prefs (server + localStorage).
import { getPref, setPref } from './prefs.svelte';

export type PaneKey = 'nav' | 'list' | 'details';

export const PANE_LIMITS: Record<PaneKey, { min: number; max: number; def: number }> = {
  nav: { min: 168, max: 320, def: 208 },
  list: { min: 200, max: 520, def: 280 },
  details: { min: 280, max: 640, def: 360 },
};

export type ColKey = 'num' | 'author' | 'series' | 'genre' | 'language' | 'format' | 'size' | 'added' | 'rating' | 'libRating' | 'extRating';

export const COL_LIMITS: Record<ColKey, { min: number; max: number; def: number }> = {
  num: { min: 32, max: 90, def: 40 },
  author: { min: 80, max: 420, def: 170 },
  series: { min: 80, max: 420, def: 170 },
  genre: { min: 80, max: 320, def: 140 },
  language: { min: 48, max: 120, def: 64 },
  format: { min: 48, max: 120, def: 64 },
  size: { min: 56, max: 140, def: 72 },
  added: { min: 72, max: 160, def: 92 },
  rating: { min: 64, max: 160, def: 70 },
  libRating: { min: 40, max: 120, def: 44 },
  extRating: { min: 64, max: 160, def: 80 },
};

export function clampWidth(v: number, lim: { min: number; max: number }): number {
  return Math.round(Math.min(lim.max, Math.max(lim.min, v)));
}

/** A stored width: a finite number (a string from an old or hand-edited prefs blob is
 *  parsed), clamped to the current limits; `null` when unusable. */
function storedWidth(v: unknown, lim: { min: number; max: number }): number | null {
  const n = typeof v === 'string' ? Number(v) : v;
  return typeof n === 'number' && Number.isFinite(n) ? clampWidth(n, lim) : null;
}

export function paneWidth(k: PaneKey): number {
  const all = getPref<Record<string, unknown> | null>('panes', {});
  return storedWidth(all && typeof all === 'object' ? all[k] : null, PANE_LIMITS[k]) ?? PANE_LIMITS[k].def;
}

export function setPaneWidth(k: PaneKey, w: number | null) {
  const cur = { ...(getPref<Record<string, number> | null>('panes', {}) ?? {}) };
  if (w === null || !Number.isFinite(w)) delete cur[k];
  else cur[k] = clampWidth(w, PANE_LIMITS[k]);
  setPref('panes', cur);
}

export function colWidth(k: ColKey): number {
  const all = getPref<Record<string, unknown> | null>('columns', {});
  return storedWidth(all && typeof all === 'object' ? all[k] : null, COL_LIMITS[k]) ?? COL_LIMITS[k].def;
}

export function setColWidth(k: ColKey, w: number | null) {
  const cur = { ...(getPref<Record<string, number> | null>('columns', {}) ?? {}) };
  if (w === null || !Number.isFinite(w)) delete cur[k];
  else cur[k] = clampWidth(w, COL_LIMITS[k]);
  setPref('columns', cur);
}

export function detailsCollapsed(): boolean {
  return getPref<boolean>('detailsCollapsed', false);
}

export function setDetailsCollapsed(v: boolean) {
  setPref('detailsCollapsed', v);
}
