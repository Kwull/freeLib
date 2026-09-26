import { api } from '../api/client';
import type { FollowList } from '../api/types';

// The user's followed authors and series, per library (loaded on first use).
export const followsState = $state<{ byLib: Record<number, FollowList>; loaded: Record<number, boolean> }>({
  byLib: {},
  loaded: {},
});

const loading = new Set<number>();

export function loadFollows(lib: number, force = false) {
  if ((followsState.loaded[lib] && !force) || loading.has(lib)) return;
  loading.add(lib);
  api.follows(lib)
    .then((f) => setFollows(lib, f))
    .catch(() => {})
    .finally(() => loading.delete(lib));
}

export function setFollows(lib: number, f: FollowList) {
  followsState.byLib = { ...followsState.byLib, [lib]: f };
  followsState.loaded = { ...followsState.loaded, [lib]: true };
}

export function isFollowed(lib: number, kind: 'author' | 'series', id: number): boolean {
  const f = followsState.byLib[lib];
  if (!f) return false;
  return (kind === 'author' ? f.authors : f.series).some((x) => x.id === id);
}
