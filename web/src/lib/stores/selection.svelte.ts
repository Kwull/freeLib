// Book selection, kept in memory only and scoped per library id.
const perLibrary = new Map<number, Set<number>>();

export const selectionState = $state<{ libId: number | null; version: number }>({ libId: null, version: 0 });

function setFor(libId: number): Set<number> {
  let s = perLibrary.get(libId);
  if (!s) { s = new Set(); perLibrary.set(libId, s); }
  return s;
}

export function setActiveLibrary(libId: number) {
  selectionState.libId = libId;
}

// perLibrary's Map/Set are plain JS, not $state, so reads must touch
// selectionState.version to register as a dependency for Svelte's reactivity —
// otherwise callers that use these in templates/derived never re-run when a
// checkbox toggles.
export function selectedIds(libId: number): number[] {
  selectionState.version;
  return [...setFor(libId)];
}

export function isSelected(libId: number, id: number): boolean {
  selectionState.version;
  return setFor(libId).has(id);
}

export function selectedCount(libId: number): number {
  selectionState.version;
  return setFor(libId).size;
}

function bump() { selectionState.version++; }

export function toggle(libId: number, id: number, on?: boolean) {
  const s = setFor(libId);
  const next = on ?? !s.has(id);
  if (next) s.add(id); else s.delete(id);
  bump();
}

export function toggleMany(libId: number, ids: number[], on: boolean) {
  const s = setFor(libId);
  for (const id of ids) { if (on) s.add(id); else s.delete(id); }
  bump();
}

export function clear(libId: number) {
  setFor(libId).clear();
  bump();
}
