import { api, errorText } from '../api/client';
import type { Library } from '../api/types';

export const librariesState = $state<{
  items: Library[]; loaded: boolean; currentId: number | null;
  /** The last `GET /libraries` failed (the server's message); cleared by the next success. */
  error: string | null;
}>({
  items: [],
  loaded: false,
  currentId: null,
  error: null,
});

export async function loadLibraries() {
  let items: Library[];
  try {
    items = await api.libraries();
  } catch (e) {
    librariesState.error = errorText(e);
    throw e;
  }
  librariesState.items = items;
  librariesState.loaded = true;
  librariesState.error = null;
  if (librariesState.currentId === null) {
    const def = items.find((l) => l.isDefault) ?? items[0];
    librariesState.currentId = def ? def.id : null;
  }
  return items;
}

export function currentLibrary(): Library | null {
  return librariesState.items.find((l) => l.id === librariesState.currentId) ?? null;
}

/**
 * Whether the library has a catalog to browse. A library without one is being imported for the
 * first time, rebuilt after an update (`status.reason === 'upgrade'`), failed, or was never
 * imported; a re-import of a browsable library keeps the old catalog until it finishes.
 */
export function isBrowsable(lib: Library): boolean {
  return !!lib.importedAt;
}

export function setCurrentLibrary(id: number) {
  librariesState.currentId = id;
}

export function upsertLibrary(lib: Library) {
  const i = librariesState.items.findIndex((l) => l.id === lib.id);
  if (i >= 0) librariesState.items[i] = lib;
  else librariesState.items.push(lib);
  if (librariesState.currentId === null) librariesState.currentId = lib.id;
}

export function removeLibrary(id: number) {
  librariesState.items = librariesState.items.filter((l) => l.id !== id);
  if (librariesState.currentId === id) {
    const def = librariesState.items.find((l) => l.isDefault) ?? librariesState.items[0];
    librariesState.currentId = def ? def.id : null;
  }
}
