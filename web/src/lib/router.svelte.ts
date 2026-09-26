// Small history-based router — no SvelteKit needed for this SPA's route set.

export const routerState = $state<{ path: string; search: string }>({
  path: location.pathname,
  search: location.search,
});

export function navigate(to: string, opts?: { replace?: boolean }) {
  const url = new URL(to, location.origin);
  if (opts?.replace) history.replaceState({}, '', url);
  else history.pushState({}, '', url);
  routerState.path = url.pathname;
  routerState.search = url.search;
}

window.addEventListener('popstate', () => {
  routerState.path = location.pathname;
  routerState.search = location.search;
});

// Intercept plain in-app link clicks so we don't reload the page.
document.addEventListener('click', (e) => {
  const a = (e.target as HTMLElement)?.closest?.('a[data-link]') as HTMLAnchorElement | null;
  if (!a) return;
  if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
  const href = a.getAttribute('href');
  if (!href || href.startsWith('http') || href.startsWith('//')) return;
  e.preventDefault();
  navigate(href);
});

export type Route =
  | { name: 'home' }
  | { name: 'new'; lib: number }
  | { name: 'authors'; lib: number; id: number | null }
  | { name: 'series'; lib: number; id: number | null }
  | { name: 'genres'; lib: number; id: number | null }
  | { name: 'shelvesIndex'; lib: number }
  | { name: 'shelf'; lib: number; id: number }
  | { name: 'search'; lib: number; q: string }
  | { name: 'book'; lib: number; id: number }
  | { name: 'read'; lib: number; id: number }
  | { name: 'libraries' }
  | { name: 'settings'; section: string | null }
  | { name: 'login' }
  | { name: 'oauthConsent'; request: string | null; error: string | null }
  | { name: 'notFound' };

export function parseRoute(path: string, search: string): Route {
  const segs = path.split('/').filter(Boolean);
  const q = new URLSearchParams(search);
  if (segs.length === 0) return { name: 'home' };
  if (segs[0] === 'login') return { name: 'login' };
  if (segs[0] === 'oauth' && segs[1] === 'consent') return { name: 'oauthConsent', request: q.get('request'), error: q.get('error') };
  if (segs[0] === 'libraries') return { name: 'libraries' };
  if (segs[0] === 'settings') return { name: 'settings', section: segs[1] ?? null };
  if (segs[0] === 'l' && segs[1]) {
    const lib = Number(segs[1]);
    const rest = segs[2];
    if (!rest) return { name: 'authors', lib, id: null };
    if (rest === 'new') return { name: 'new', lib };
    if (rest === 'authors') return { name: 'authors', lib, id: segs[3] ? Number(segs[3]) : null };
    if (rest === 'series') return { name: 'series', lib, id: segs[3] ? Number(segs[3]) : null };
    if (rest === 'genres') return { name: 'genres', lib, id: segs[3] ? Number(segs[3]) : null };
    if (rest === 'shelves' && segs[3]) return { name: 'shelf', lib, id: Number(segs[3]) };
    if (rest === 'shelves') return { name: 'shelvesIndex', lib };
    if (rest === 'search') return { name: 'search', lib, q: q.get('q') ?? '' };
    if (rest === 'book' && segs[3]) return { name: 'book', lib, id: Number(segs[3]) };
    if (rest === 'read' && segs[3]) return { name: 'read', lib, id: Number(segs[3]) };
  }
  return { name: 'notFound' };
}

export function currentRoute(): Route {
  return parseRoute(routerState.path, routerState.search);
}
