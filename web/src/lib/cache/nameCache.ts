import { openDB, type IDBPDatabase } from 'idb';
import { api } from '../api/client';
import type { NameListResponse } from '../api/types';

let dbPromise: Promise<IDBPDatabase> | null = null;

function db() {
  if (!dbPromise) {
    dbPromise = openDB('freelib-cache', 1, {
      upgrade(d) {
        d.createObjectStore('nameLists'); // key: `${lib}:${kind}` -> { version, data }
      },
    });
  }
  return dbPromise;
}

/**
 * Fetches the compact authors/series list once per catalogVersion and caches it
 * in IndexedDB, so revisits (and other tabs) skip the network + JSON parse.
 */
export async function loadNameList(
  lib: number,
  kind: 'authors' | 'series',
  catalogVersion: number,
): Promise<NameListResponse> {
  const key = `${lib}:${kind}`;
  try {
    const cached = await (await db()).get('nameLists', key);
    if (cached && cached.version === catalogVersion) return cached.data as NameListResponse;
  } catch { /* IndexedDB unavailable — fall through to network */ }

  const data = kind === 'authors' ? await api.authors(lib, catalogVersion) : await api.series(lib, catalogVersion);
  try {
    await (await db()).put('nameLists', { version: catalogVersion, data }, key);
  } catch { /* best effort */ }
  return data;
}
