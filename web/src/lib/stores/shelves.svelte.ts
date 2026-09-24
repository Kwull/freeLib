import { api } from '../api/client';
import type { Shelf } from '../api/types';

export const shelvesState = $state<{ items: Shelf[]; loaded: boolean }>({ items: [], loaded: false });

export async function loadShelves() {
  shelvesState.items = await api.shelves();
  shelvesState.loaded = true;
}

export async function createShelf(name: string, color: string) {
  const s = await api.createShelf(name, color);
  shelvesState.items.push(s);
  return s;
}

export async function deleteShelf(id: number) {
  await api.deleteShelf(id);
  shelvesState.items = shelvesState.items.filter((s) => s.id !== id);
}

export async function setBooksOnShelf(shelfId: number, library: number, books: number[], add: boolean) {
  const shelf = await api.shelfBooks(shelfId, library, books, add);
  const i = shelvesState.items.findIndex((s) => s.id === shelfId);
  if (i >= 0) shelvesState.items[i] = shelf;
}
