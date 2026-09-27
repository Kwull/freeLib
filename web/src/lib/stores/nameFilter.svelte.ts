// The authors / series filter text, per library and list (`${lib}:authors`). It outlives the
// page: coming back to Authors shows the same filter, and what is typed into the placeholder
// filter box while the library or the list still loads (BrowseSkeleton) is what the real box
// starts with — never lost, never cleared by a load.
export const nameFilterState = $state<{ values: Record<string, string>; focusKey: string | null }>({
  values: {},
  focusKey: null,
});

export function nameFilterKey(lib: number | null, kind: 'authors' | 'series'): string {
  return `${lib ?? 'current'}:${kind}`;
}

export function getNameFilter(key: string): string {
  return nameFilterState.values[key] ?? '';
}

export function setNameFilter(key: string, value: string) {
  if ((nameFilterState.values[key] ?? '') === value) return;
  nameFilterState.values[key] = value;
}
