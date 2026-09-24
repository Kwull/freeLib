import { api } from '../api/client';
import type { Library } from '../api/types';

export const librariesState = $state<{ items: Library[]; loaded: boolean; currentId: number | null }>({
  items: [],
  loaded: false,
  currentId: null,
});

export async function loadLibraries() {
  const items = await api.libraries();
  librariesState.items = items;
  librariesState.loaded = true;
  if (librariesState.currentId === null) {
    const def = items.find((l) => l.isDefault) ?? items[0];
    librariesState.currentId = def ? def.id : null;
  }
  return items;
}

export function currentLibrary(): Library | null {
  return librariesState.items.find((l) => l.id === librariesState.currentId) ?? null;
}

export function setCurrentLibrary(id: number) {
  librariesState.currentId = id;
}

export function upsertLibrary(lib: Library) {
  const i = librariesState.items.findIndex((l) => l.id === lib.id);
  if (i >= 0) librariesState.items[i] = lib;
  else librariesState.items.push(lib);
}

export function removeLibrary(id: number) {
  librariesState.items = librariesState.items.filter((l) => l.id !== id);
}
