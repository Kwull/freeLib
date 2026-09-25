// Per-user UI preferences (pane and column widths, view options, last library…), stored on
// the server via GET/PUT /me/prefs so they follow the user across browsers. localStorage
// keeps a copy so the first paint already uses them; writes are merged and debounced.
import { api } from '../api/client';
import { isLoggedIn } from './session.svelte';

const LS_KEY = 'freelib.prefs';

function readLocal(): Record<string, unknown> {
  try {
    const v = JSON.parse(localStorage.getItem(LS_KEY) ?? '{}');
    return v && typeof v === 'object' ? v : {};
  } catch {
    return {};
  }
}

function writeLocal(v: Record<string, unknown>) {
  try { localStorage.setItem(LS_KEY, JSON.stringify(v)); } catch { /* ignore */ }
}

export const prefsState = $state<{ values: Record<string, unknown>; loaded: boolean }>({
  values: readLocal(),
  loaded: false,
});

/** Keys changed in this page since load: a late server response must not overwrite them. */
const dirty = new Set<string>();

export async function loadPrefs(): Promise<Record<string, unknown>> {
  try {
    const server = await api.prefs();
    const merged = { ...prefsState.values };
    for (const [k, v] of Object.entries(server ?? {})) if (!dirty.has(k)) merged[k] = v;
    prefsState.values = merged;
    writeLocal(merged);
  } catch { /* best effort: keep the local copy */ }
  prefsState.loaded = true;
  return prefsState.values;
}

let timer: ReturnType<typeof setTimeout> | undefined;
function scheduleSave() {
  if (timer) clearTimeout(timer);
  timer = setTimeout(() => {
    timer = undefined;
    if (!isLoggedIn()) return;
    api.setPrefs(prefsState.values).catch(() => {});
  }, 800);
}

export function getPref<T>(key: string, fallback: T): T {
  const v = prefsState.values[key];
  return v === undefined || v === null ? fallback : (v as T);
}

export function setPref(key: string, value: unknown) {
  dirty.add(key);
  prefsState.values = { ...prefsState.values, [key]: value };
  writeLocal(prefsState.values);
  scheduleSave();
}
