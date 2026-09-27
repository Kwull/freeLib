import type { Connect } from 'vite';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { store, catalog, createJob, runJobProgress, runSendJob, broadcast } from './store';
import type { JobItem } from '../src/lib/api/types';
import { placeholderCover } from './covers';
import { scenarioOf, handleScenario, handleMockControl, sendSlowly } from './scenario';
import { normalize } from './normalize';
import { genreName } from './names';
import type { Book, BookDetail, AuthorRef, SeriesRef, Isbn } from '../src/lib/api/types';
import { ANTHOLOGY_MIN_AUTHORS, type MockBook, type MockLibrary } from './gen';
import {
  correct, dismissed, editionNote, editionsOf, followsOf, groupBooks, highlightWords, homeOf, matchTier,
  queryTokens, recordHistory, type Grouped,
} from './find';

// A small, real two-chapter Russian EPUB (see fixtures/build-epub.mjs) served
// for `format=epub` so the in-browser reader has actual content to render,
// instead of the placeholder byte string used for other formats.
const FIXTURES_DIR = path.dirname(fileURLToPath(import.meta.url)) + '/fixtures';
let sampleEpub: Buffer | null = null;
function getSampleEpub(): Buffer {
  if (!sampleEpub) sampleEpub = readFileSync(path.join(FIXTURES_DIR, 'sample-ru.epub'));
  return sampleEpub;
}

const COOKIE = 'freelib_session';

function send(res: ServerResponse, status: number, body?: unknown, headers?: Record<string, string>) {
  res.statusCode = status;
  for (const [k, v] of Object.entries(headers ?? {})) res.setHeader(k, v);
  if (body === undefined) { res.end(); return; }
  res.setHeader('Content-Type', 'application/json; charset=utf-8');
  res.end(JSON.stringify(body));
}

function fail(res: ServerResponse, status: number, code: string, message: string) {
  send(res, status, { error: code, message });
}

async function readBody(req: IncomingMessage): Promise<any> {
  return new Promise((resolve) => {
    let data = '';
    req.on('data', (c) => (data += c));
    req.on('end', () => {
      try { resolve(data ? JSON.parse(data) : {}); } catch { resolve({}); }
    });
  });
}

function toAuthorRefs(lib: MockLibrary, ids: number[]): AuthorRef[] {
  return ids.map((id) => ({ id, name: lib.authors[id - 1]?.name ?? '?' }));
}
function toSeriesRef(lib: MockLibrary, id: number | null): SeriesRef | null {
  if (!id) return null;
  const s = lib.series[id - 1];
  return s ? { id, name: s.name } : null;
}

function bookRating(libId: number, id: number): number {
  return store.ratings.get(`${libId}:${id}`) ?? 0;
}
function bookShelves(libId: number, key: string): number[] {
  const out: number[] = [];
  for (const [sid, set] of store.shelfBooks) if (set.has(`${libId}:${key}`)) out.push(sid);
  return out;
}

// Deterministic mock ratings: a library (INPX) rating for ~40% of books, an Open Library
// rating for ~35%, and an age estimate from the (mock) genres like the server's heuristic.
function hash(n: number): number {
  let x = (n * 2654435761) >>> 0;
  x ^= x >>> 13; x = Math.imul(x, 0x5bd1e995) >>> 0; x ^= x >>> 15;
  return x >>> 0;
}
export function libRating(b: MockBook): number {
  if (b.key.startsWith('demo:')) return [5, 5, 4, 4, 5, 4, 3, 4, 3, 5, 3, 2, 4, 3, 3][b.id - 1] ?? 4;
  const h = hash(b.id);
  return h % 10 < 4 ? 1 + ((h >>> 8) % 5) : 0;
}
export function extRating(b: MockBook): { avg: number; votes: number } | null {
  if (b.key.startsWith('demo:')) return { avg: [4.2, 4.0, 4.3, 4.2, 4.4, 4.1, 3.9, 4.0, 3.8, 3.9, 3.5, 3.4, 3.7, 3.6, 3.5][b.id - 1] ?? 4, votes: [950, 610, 1210, 480, 2104, 390, 270, 220, 180, 830, 120, 95, 60, 18, 12][b.id - 1] ?? 10 };
  const h = hash(b.id + 7919);
  if (h % 100 >= 35) return null;
  return { avg: Math.round((2.5 + ((h >>> 7) % 250) / 100) * 100) / 100, votes: 1 + ((h >>> 3) % 400) };
}
export function kidsAge(b: MockBook): number | null {
  // mock genres: 18 Children's, 19 Children's Prose, 20 Children's Adventure, 10 Thriller
  if (b.genreIds.includes(10)) return 16;
  if (b.genreIds.includes(20)) return 12;
  if (b.genreIds.includes(19) || b.genreIds.includes(18)) return b.id % 3 === 0 ? 0 : 6;
  return b.key.startsWith('demo:') ? 12 : null;
}

function toBook(lib: MockLibrary, b: MockBook): Book {
  return {
    id: b.id, key: b.key, title: b.title,
    authors: toAuthorRefs(lib, b.authorIds),
    series: toSeriesRef(lib, b.seriesId), serno: b.serno,
    genres: b.genreIds, lang: b.lang, ext: b.ext, size: b.size, date: b.date, deleted: b.deleted,
    rating: bookRating(lib.id, b.id), shelves: bookShelves(lib.id, b.key),
    libRating: libRating(b), extRating: extRating(b), kidsAge: kidsAge(b),
  };
}

/** The server's rating filters and sorts (`sort=my|lib|ext`, `minMy`, …). */
function rateFilterSort(lib: MockLibrary, items: MockBook[], sp: URLSearchParams): MockBook[] | string {
  const n = (k: string) => { const v = sp.get(k); return v === null || v === '' ? 0 : Number(v); };
  const minMy = n('minMy'), minLib = n('minLib'), minExt = n('minExt'), minVotes = n('minExtVotes');
  if ([minMy, minLib, minExt, minVotes].some((x) => Number.isNaN(x)) || minMy > 5 || minLib > 5 || minExt > 5) return 'invalid rating filter';
  const unrated = sp.get('unratedByMe') === '1';
  const kids = sp.get('kidsMaxAge');
  let out = items.filter((b) => {
    const my = bookRating(lib.id, b.id);
    if (minMy && my < minMy) return false;
    if (unrated && my > 0) return false;
    if (minLib && libRating(b) < minLib) return false;
    const e = extRating(b);
    if ((minExt || minVotes) && (!e || e.avg < minExt || e.votes < minVotes)) return false;
    if (kids !== null && kids !== '') { const a = kidsAge(b); if (a === null || a > Number(kids)) return false; }
    return true;
  });
  const sort = sp.get('sort');
  const key = (b: MockBook) => sort === 'my' ? bookRating(lib.id, b.id) : sort === 'lib' ? libRating(b)
    : sort === 'ext' ? (extRating(b)?.avg ?? 0) * 1e6 + (extRating(b)?.votes ?? 0) : 0;
  if (sort === 'my' || sort === 'lib' || sort === 'ext') {
    out = out.map((b, i) => ({ b, i, k: key(b) })).sort((x, y) => y.k - x.k || x.i - y.i).map((x) => x.b);
  }
  return out;
}

/** A grouped list row: the best copy, with `editions` when the work has several. */
function toRow(lib: MockLibrary, g: Grouped): Book {
  const b = toBook(lib, g.best);
  if (g.members.length > 1) b.editions = { count: g.members.length, ids: g.members.map((m) => m.id) };
  return b;
}

/** A valid ISBN (978-5-…) derived from the book id, like one read from <publish-info>. */
function mockIsbn(b: MockBook): Isbn | null {
  if (b.ext !== 'fb2' || hash(b.id + 7) % 4 === 0) return null;
  const body = String(10_000_000 + (b.id * 7919) % 90_000_000).padStart(8, '0');
  const d12 = `9785${body}`;
  const s13 = [...d12].reduce((n, c, i) => n + Number(c) * (i % 2 ? 3 : 1), 0);
  const c13 = (10 - (s13 % 10)) % 10;
  const d9 = `5${body}`;
  const s10 = [...d9].reduce((n, c, i) => n + Number(c) * (10 - i), 0);
  const c10 = (11 - (s10 % 11)) % 11;
  return {
    isbn13: `${d12}${c13}`, isbn10: `${d9}${c10 === 10 ? 'X' : c10}`,
    display: `978-5-${body.slice(0, 3)}-${body.slice(3)}-${c13}`,
  };
}

function toDetail(lib: MockLibrary, b: MockBook): BookDetail {
  const isbn = mockIsbn(b);
  const e = extRating(b);
  const h = hash(b.id + 31);
  return {
    ...toBook(lib, b),
    extRatingInfo: e
      ? { source: 'openlibrary', status: 'found', average: e.avg, count: e.votes, workKey: `/works/OL${h % 900000}W`, url: `https://openlibrary.org/works/OL${h % 900000}W`, fetchedAt: '2026-09-01T10:00:00Z' }
      : h % 3 === 0 ? { source: 'openlibrary', status: 'not_found', average: null, count: 0, workKey: null, url: null, fetchedAt: '2026-09-01T10:00:00Z' } : null,
    annotation: b.annotation ?? `<p>The annotation for &laquo;${b.title}&raquo; will appear here after the file is first opened, and will be cached on the server.</p>`,
    hasCover: true,
    file: `${b.key.split(':')[0]}-archive.zip / ${b.id}.${b.ext}`,
    keywords: b.keywords ?? '',
    formats: ['original', 'epub', b.ext !== 'epub' ? 'epub' : 'fb2', 'kepub', 'azw3'].filter((v, i, a) => a.indexOf(v) === i),
    isbn: isbn ? [isbn] : [],
    isbnRaw: null,
    publisher: isbn ? ['Эксмо', 'АСТ', 'Азбука', 'Penguin'][b.id % 4] : null,
    publishYear: isbn ? b.date.slice(0, 4) : null,
  };
}

function paginate<T>(items: T[], cursor: string | null, limit: number): { page: T[]; next: string | null } {
  const start = cursor ? Number(cursor) : 0;
  const page = items.slice(start, start + limit);
  const next = start + limit < items.length ? String(start + limit) : null;
  return { page, next };
}

function matchLib(req: IncomingMessage, pattern: RegExp): RegExpMatchArray | null {
  const url = (req.url ?? '').split('?')[0];
  return url.match(pattern);
}

/** `/h/<token>[/cover|/file]`: the phone page of a hand-off link (a simplified copy of the server's). */
function handoffPage(req: IncomingMessage, res: ServerResponse, path: string) {
  const [, , token, what] = path.split('/');
  const h = store.handoffs.get(token ?? '');
  const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
  if (!h || h.expires < Date.now() || h.uses >= 3) {
    res.statusCode = 410;
    res.setHeader('Content-Type', 'text/html; charset=utf-8');
    res.end('<!doctype html><title>Link expired</title><h1>This link has expired</h1>');
    return;
  }
  const lib = catalog(h.lib);
  const b = lib.bookById.get(h.book)!;
  if (what === 'cover') {
    res.setHeader('Content-Type', 'image/svg+xml');
    res.end(placeholderCover(b.title, 'thumb'));
    return;
  }
  if (what === 'file') {
    h.uses++;
    const buf = getSampleEpub();
    res.setHeader('Content-Type', 'application/epub+zip');
    res.setHeader('Content-Disposition', `attachment; filename="${encodeURIComponent(b.title)}.epub"; filename*=UTF-8''${encodeURIComponent(b.title)}.epub`);
    res.setHeader('Cache-Control', 'no-store');
    res.end(buf);
    return;
  }
  const ios = /iPhone|iPad|iPod/.test(String(req.headers['user-agent'] ?? ''));
  const authors = b.authorIds.map((a) => lib.authors[a - 1].name).join(', ');
  res.setHeader('Content-Type', 'text/html; charset=utf-8');
  res.setHeader('Cache-Control', 'no-store');
  res.end(`<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${esc(b.title)}</title>
<style>body{margin:0;min-height:100vh;background:#f6f3ee;font:16px/1.45 -apple-system,sans-serif;display:flex;align-items:center;justify-content:center;padding:24px 16px}
main{max-width:420px;width:100%;background:#fff;border-radius:18px;padding:28px 22px;text-align:center}img{max-width:62%;border-radius:6px}
a.btn{display:block;margin:22px 0 10px;padding:15px;border-radius:12px;background:#1f5f5b;color:#fff;font-weight:600;text-decoration:none}</style></head>
<body><main><img src="/h/${token}/cover" alt=""><h1>${esc(b.title)}</h1><p>${esc(authors)}</p>
<a class="btn" href="/h/${token}/file">${ios ? 'Open in Books' : 'Download'}</a><p>${h.format.toUpperCase()}</p></main></body></html>`);
}

export function installMockApi(server: Connect.Server) {
  server.use(async (req, res, next) => {
    const url = new URL(req.url ?? '/', 'http://localhost');
    const path = url.pathname;
    const method = req.method ?? 'GET';
    if (handleMockControl(url, res)) return;
    if (path.startsWith('/h/')) return handoffPage(req, res, path);
    if (!path.startsWith('/api/v1') && !path.startsWith('/opds')) return next();

    // simulate small network latency for realism
    await new Promise((r) => setTimeout(r, 15 + Math.random() * 25));

    // simulated server states (see mock/scenario.ts)
    const sc = scenarioOf(req);
    if (sc && await handleScenario(sc, req, res, path, () => store.libraries, { id: 1, username: 'admin', role: 'admin' })) return;

    try {
      // ---- Session --------------------------------------------------
      if (path === '/api/v1/session' && method === 'GET') {
        return send(res, 200, { user: { id: 1, username: 'admin', role: 'admin' }, openMode: true, auth: { password: true, oidc: null } });
      }
      if (path === '/api/v1/login' && method === 'POST') {
        const body = await readBody(req);
        if (body.username === 'admin' && body.password === 'admin') {
          res.setHeader('Set-Cookie', `${COOKIE}=mock; Path=/; HttpOnly; SameSite=Lax`);
          return send(res, 200, { user: { id: 1, username: 'admin', role: 'admin' } });
        }
        return fail(res, 401, 'unauthorized', 'Invalid username or password');
      }
      if (path === '/api/v1/logout' && method === 'POST') return send(res, 204);

      // ---- Libraries --------------------------------------------------
      if (path === '/api/v1/libraries' && method === 'GET') return send(res, 200, store.libraries);
      if (path === '/api/v1/libraries' && method === 'POST') {
        const body = await readBody(req);
        const id = store.nextLibraryId++;
        catalog(id);
        const lib = {
          id, name: body.name, path: body.path, inpx: body.inpx ?? null,
          firstAuthorOnly: !!body.firstAuthorOnly, skipDeleted: !!body.skipDeleted, isDefault: !!body.isDefault,
          bookCount: 0, authorCount: 0, seriesCount: 0, importedAt: null, catalogVersion: 0, newSinceLastVisit: 0,
          status: { state: 'idle' as const }, opdsUrl: `/opds/${id}`,
        };
        store.libraries.push(lib);
        if (body.inpx) {
          const job = createJob('import', `Import ${body.name}`);
          lib.status = { state: 'importing', progress: 0 };
          broadcast('library', lib);
          runJobProgress(job, {
            onDone: () => {
              const c = catalog(id);
              lib.bookCount = c.books.filter((b) => !b.deleted).length; lib.authorCount = c.authorRows.length; lib.seriesCount = c.seriesRows.length;
              lib.importedAt = new Date().toISOString(); lib.catalogVersion = 1;
              lib.status = { state: 'idle' };
              broadcast('library', lib);
            },
          });
        }
        return send(res, 200, lib);
      }
      let m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)$/);
      if (m && method === 'PATCH') {
        const lib = store.libraries.find((l) => l.id === Number(m![1]));
        if (!lib) return fail(res, 404, 'not_found', 'Library not found');
        Object.assign(lib, await readBody(req));
        return send(res, 200, lib);
      }
      if (m && method === 'DELETE') {
        const id = Number(m[1]);
        store.libraries = store.libraries.filter((l) => l.id !== id);
        store.catalogs.delete(id);
        return send(res, 204);
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/import$/);
      if (m && method === 'POST') {
        const lib = store.libraries.find((l) => l.id === Number(m![1]));
        if (!lib) return fail(res, 404, 'not_found', 'Library not found');
        if (lib.status.state === 'importing') return fail(res, 409, 'conflict', 'Import already running');
        const body = await readBody(req);
        const job = createJob('import', `${body.mode === 'full' ? 'Full' : 'Incremental'} import · ${lib.name}`);
        lib.status = { state: 'importing', progress: 0 };
        broadcast('library', lib);
        runJobProgress(job, { onDone: () => { lib.status = { state: 'idle' }; lib.catalogVersion++; broadcast('library', lib); } });
        return send(res, 200, job);
      }
      if (path === '/api/v1/fs' && method === 'GET') {
        const p = url.searchParams.get('path') ?? '';
        const fake: Record<string, { name: string; dir: boolean; size: number }[]> = {
          '': [{ name: 'flibusta', dir: true, size: 0 }, { name: 'home', dir: true, size: 0 }],
          flibusta: [{ name: 'flibusta.inpx', dir: false, size: 45_000_000 }, { name: 'f.fb2-000001-005000.zip', dir: false, size: 500_000_000 }],
          home: [{ name: 'collection.inpx', dir: false, size: 2_000_000 }],
        };
        const entries = fake[p] ?? [];
        return send(res, 200, { path: p, parent: p ? '' : null, entries });
      }

      // ---- Browsing -----------------------------------------------------
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/authors$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const body = { version: 1, columns: ['id', 'name', 'count'], rows: lib.authorRows, letters: lib.authorLetters };
        if (sc?.flags.has('slow')) return sendSlowly(res, body, sc.slow);
        return send(res, 200, body, { 'Cache-Control': url.searchParams.has('v') ? 'public, max-age=31536000, immutable' : 'no-cache' });
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/series$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const body = { version: 1, columns: ['id', 'name', 'count'], rows: lib.seriesRows, letters: lib.seriesLetters };
        if (sc?.flags.has('slow')) return sendSlowly(res, body, sc.slow);
        return send(res, 200, body);
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/genres$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const lang = url.searchParams.get('lang') ?? 'en';
        return send(res, 200, lib.genres.map((g) => ({
          id: g.id, name: genreName(g, lang), parent: g.parent, count: g.count,
        })));
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/books$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const author = url.searchParams.get('author');
        const series = url.searchParams.get('series');
        const genre = url.searchParams.get('genre');
        const shelf = url.searchParams.get('shelf');
        const since = url.searchParams.get('since');
        const lang = url.searchParams.get('lang');
        const ext = url.searchParams.get('ext');
        const showDeleted = url.searchParams.get('deleted') === '1';
        const cursor = url.searchParams.get('cursor');
        const limit = Math.min(Number(url.searchParams.get('limit') ?? 2000) || 2000, 5000);

        let items: MockBook[];
        if (author) {
          items = (lib.booksByAuthor.get(Number(author)) ?? []).map((id) => lib.bookById.get(id)!);
          items = items.slice().sort((a, b) => {
            const as = a.seriesId ? lib.series[a.seriesId - 1].sortKey : '￿';
            const bs = b.seriesId ? lib.series[b.seriesId - 1].sortKey : '￿';
            if (as !== bs) return as < bs ? -1 : 1;
            if ((a.serno ?? 0) !== (b.serno ?? 0)) return (a.serno ?? 0) - (b.serno ?? 0);
            return a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0;
          });
        } else if (series) {
          items = (lib.booksBySeries.get(Number(series)) ?? []).map((id) => lib.bookById.get(id)!);
          items = items.slice().sort((a, b) => {
            if ((a.serno ?? 0) !== (b.serno ?? 0)) return (a.serno ?? 0) - (b.serno ?? 0);
            return a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0;
          });
        } else if (genre) {
          items = lib.books.filter((b) => b.genreIds.includes(Number(genre)));
          items = items.slice().sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : a.sortKey < b.sortKey ? -1 : 1));
        } else if (shelf) {
          const set = store.shelfBooks.get(Number(shelf)) ?? new Set();
          items = lib.books.filter((b) => set.has(`${lib.id}:${b.key}`));
          items = items.slice().sort((a, b) => (a.date < b.date ? 1 : -1));
        } else if (since) {
          items = lib.books.filter((b) => b.date > since);
          items = items.slice().sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : a.sortKey < b.sortKey ? -1 : 1));
        } else {
          return fail(res, 400, 'bad_request', 'One of author, series, genre, shelf, since is required');
        }
        const q = normalize(url.searchParams.get('q') ?? '').split(' ').filter(Boolean);
        if (q.length) {
          // like the server: every word is a prefix of a word of the title, authors or series
          items = items.filter((b) => {
            const hay = normalize(`${b.title} ${b.authorIds.map((a) => lib.authors[a - 1].name).join(' ')} ${b.seriesId ? lib.series[b.seriesId - 1].name : ''}`).split(' ');
            return q.every((w) => hay.some((h) => h.startsWith(w)));
          });
        }
        if (lang) items = items.filter((b) => b.lang === lang);
        if (ext) items = items.filter((b) => b.ext === ext);
        if (!showDeleted) items = items.filter((b) => !b.deleted);
        const rated = rateFilterSort(lib, items, url.searchParams);
        if (typeof rated === 'string') return fail(res, 400, 'bad_request', rated);
        items = rated;

        if (url.searchParams.get('group') === '1') {
          const groups = groupBooks(items);
          // with a rating sort a work is placed by its best copy's rating (what the row shows)
          const s = url.searchParams.get('sort');
          if (s === 'my' || s === 'lib' || s === 'ext') {
            const key = (b: MockBook) => s === 'my' ? bookRating(lib.id, b.id) : s === 'lib' ? libRating(b)
              : (extRating(b)?.avg ?? 0) * 1e6 + (extRating(b)?.votes ?? 0);
            groups.sort((x, y) => key(y.best) - key(x.best));
          }
          const { page, next } = paginate(groups, cursor, limit);
          return send(res, 200, { books: page.map((g) => toRow(lib, g)), nextCursor: next, total: groups.length });
        }
        const { page, next } = paginate(items, cursor, limit);
        return send(res, 200, { books: page.map((b) => toBook(lib, b)), nextCursor: next, total: items.length });
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/books\/(\d+)\/editions$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const eds = editionsOf(lib, Number(m[2]));
        if (!eds) return fail(res, 404, 'not_found', 'book not found');
        return send(res, 200, { best: eds[0].id, books: eds.map((b) => ({ ...toBook(lib, b), note: editionNote(b) })) });
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/home$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const d = url.searchParams.get('days');
        const days = d ? Number(d) : null;
        if (days !== null && !(days >= 1 && days <= 3650)) return fail(res, 400, 'bad_request', 'days must be 1..3650');
        const h = homeOf(lib, { days, ratings: store.ratings, fresh: !!sc?.flags.has('newuser') });
        return send(res, 200, {
          empty: h.empty,
          continueSeries: h.continueSeries.map((s) => ({ ...s, next: s.next.map((g) => toRow(lib, g)) })),
          newFromAuthors: { since: h.since, days, total: h.newTotal, books: h.newBooks.map(({ g, reason }) => ({ ...toRow(lib, g), reason })) },
          picks: h.picks.map((g) => toRow(lib, g)),
          following: h.following,
        });
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/home\/dismiss$/);
      if (m && method === 'POST') {
        const lib = catalog(Number(m[1]));
        const body = await readBody(req);
        if (!lib.series[body.series - 1]) return fail(res, 404, 'not_found', 'series not found');
        const set = dismissed.get(lib.id) ?? new Set<number>();
        if (body.dismissed === false) set.delete(body.series); else set.add(body.series);
        dismissed.set(lib.id, set);
        return send(res, 204);
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/follows$/);
      if (m && (method === 'GET' || method === 'PUT')) {
        const lib = catalog(Number(m[1]));
        const f = followsOf(lib.id);
        if (method === 'PUT') {
          const body = await readBody(req);
          if (body.kind !== 'author' && body.kind !== 'series') return fail(res, 400, 'bad_request', 'kind must be author or series');
          const exists = body.kind === 'author' ? lib.authors[body.id - 1] : lib.series[body.id - 1];
          if (!exists) return fail(res, 404, 'not_found', `${body.kind} not found`);
          const set = body.kind === 'author' ? f.authors : f.series;
          if (body.follow) set.add(body.id); else set.delete(body.id);
        }
        const rows = (ids: Set<number>, list: { id: number; name: string; bookCount: number }[]) =>
          [...ids].map((id) => ({ id, name: list[id - 1].name, count: list[id - 1].bookCount })).sort((a, b) => (a.name < b.name ? -1 : 1));
        return send(res, 200, { authors: rows(f.authors, lib.authors), series: rows(f.series, lib.series) });
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/authors\/(\d+)\/(summary|coauthors)$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const aid = Number(m[2]);
        const author = lib.authors[aid - 1];
        if (!author) return fail(res, 404, 'not_found', 'author not found');
        const live = (lib.booksByAuthor.get(aid) ?? []).map((id) => lib.bookById.get(id)!).filter((b) => !b.deleted);
        const co = new Map<number, { books: number; direct: number }>();
        for (const b of live) {
          const direct = b.authorIds.length < ANTHOLOGY_MIN_AUTHORS;
          for (const a of b.authorIds) {
            if (a === aid) continue;
            const e = co.get(a) ?? { books: 0, direct: 0 };
            e.books++; if (direct) e.direct++;
            co.set(a, e);
          }
        }
        const coauthors = [...co.entries()]
          .map(([id, e]) => ({ id, name: lib.authors[id - 1].name, books: e.books, direct: e.direct, key: lib.authors[id - 1].sortKey }))
          .sort((x, y) => y.direct - x.direct || y.books - x.books || (x.key < y.key ? -1 : x.key > y.key ? 1 : x.id - y.id));
        if (m[3] === 'coauthors') {
          return send(res, 200, { columns: ['id', 'name', 'books', 'direct'], rows: coauthors.map((c) => [c.id, c.name, c.books, c.direct]) });
        }
        const seriesCount = new Map<number, number>();
        const langs = new Map<string, number>();
        const genresCount = new Map<number, number>();
        let withoutSeries = 0, anthologies = 0, firstDate = '', lastDate = '';
        for (const b of live) {
          if (b.seriesId) seriesCount.set(b.seriesId, (seriesCount.get(b.seriesId) ?? 0) + 1); else withoutSeries++;
          langs.set(b.lang, (langs.get(b.lang) ?? 0) + 1);
          for (const g of b.genreIds) genresCount.set(g, (genresCount.get(g) ?? 0) + 1);
          if (b.authorIds.length >= ANTHOLOGY_MIN_AUTHORS) anthologies++;
          if (!firstDate || b.date < firstDate) firstDate = b.date;
          if (b.date > lastDate) lastDate = b.date;
        }
        return send(res, 200, {
          id: aid, name: author.name, count: live.length, anthologies,
          series: [...seriesCount.entries()]
            .map(([id, count]) => ({ id, name: lib.series[id - 1].name, count, key: lib.series[id - 1].sortKey }))
            .sort((x, y) => y.count - x.count || (x.key < y.key ? -1 : 1))
            .map(({ id, name, count }) => ({ id, name, count })),
          withoutSeries,
          langs: [...langs.entries()].sort((x, y) => y[1] - x[1] || (x[0] < y[0] ? -1 : 1)),
          genres: [...genresCount.entries()].sort((x, y) => y[1] - x[1] || x[0] - y[0]).slice(0, 8),
          firstDate, lastDate,
          coauthors: coauthors.filter((c) => c.direct >= 1 || c.books >= 2).slice(0, 10).map(({ id, name, books, direct }) => ({ id, name, books, direct })),
          coauthorCount: coauthors.length,
        });
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/books\/(\d+)$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const b = lib.bookById.get(Number(m[2]));
        if (!b) return fail(res, 404, 'not_found', 'Book not found');
        return send(res, 200, toDetail(lib, b));
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/books\/(\d+)\/cover$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const b = lib.bookById.get(Number(m[2]));
        if (!b) return fail(res, 404, 'not_found', 'No cover');
        const size = url.searchParams.get('size') === 'full' ? 'full' : 'thumb';
        res.setHeader('Content-Type', 'image/svg+xml');
        res.setHeader('Cache-Control', 'public, max-age=86400');
        res.end(placeholderCover(b.title, size));
        return;
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/books\/(\d+)\/file$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const b = lib.bookById.get(Number(m[2]));
        if (!b) return fail(res, 404, 'not_found', 'Book not found');
        const format = url.searchParams.get('format') ?? 'original';
        if (format === 'pdf' && !store.settings.calibre.available) return fail(res, 501, 'unsupported_format', 'Calibre not available');
        const inline = url.searchParams.get('inline') === '1';
        recordHistory(lib.id, [b.id]);
        const ext = format === 'original' ? b.ext : format;
        if (ext === 'epub') {
          const buf = getSampleEpub();
          res.setHeader('Content-Type', 'application/epub+zip');
          if (!inline) res.setHeader('Content-Disposition', `attachment; filename="${b.title}.epub"`);
          res.setHeader('Content-Length', String(buf.length));
          res.end(buf);
          return;
        }
        res.setHeader('Content-Type', inline ? 'application/epub+zip' : 'application/octet-stream');
        if (!inline) res.setHeader('Content-Disposition', `attachment; filename="${b.title}.${ext}"`);
        res.end(`Mock file content for "${b.title}" (${ext})`);
        return;
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/search$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        const q0 = url.searchParams.get('q') ?? '';
        const kind = url.searchParams.get('kind') ?? 'all';
        const t0 = Date.now();
        const run = (q: string) => {
          const tokens = queryTokens(q);
          const phrase = normalize(q);
          // tiers like the server: name/title starts with the query 3, all prefixes 2, word forms / transliteration 1
          const ranked = <T,>(items: T[], text: (x: T) => string, key: (x: T) => number) => items
            .map((x, i) => { const t = text(x); const tier = matchTier(tokens, t); return { x, i, tier: tier && normalize(t).startsWith(phrase) ? 3 : tier }; })
            .filter((r) => r.tier > 0)
            .sort((a, b) => b.tier - a.tier || key(b.x) - key(a.x) || a.i - b.i)
            .map((r) => r.x);
          const authors = kind === 'all' || kind === 'authors' ? ranked(lib.authors, (a) => a.name, (a) => a.bookCount).slice(0, 20) : [];
          const series = kind === 'all' || kind === 'series' ? ranked(lib.series, (s) => s.name, (s) => s.bookCount).slice(0, 20) : [];
          const books = kind === 'all' || kind === 'books'
            ? ranked(lib.books, (b) => `${b.title} ${b.authorIds.map((a) => lib.authors[a - 1].name).join(' ')} ${b.seriesId ? lib.series[b.seriesId - 1].name : ''} ${b.keywords ?? ''}`, () => 0)
            : [];
          return { tokens, authors, series, books };
        };
        let r = run(q0);
        let corrected: string | null = null, didYouMean: string | null = null;
        const found = (x: typeof r) => x.authors.length + x.series.length + x.books.length;
        const names = (x: typeof r) => x.authors.length + x.series.length;
        const wantsNames = kind !== 'books';
        if (url.searchParams.get('exact') !== '1' && (found(r) < 3 || (wantsNames && names(r) === 0))) {
          const fixed = correct(lib, q0);
          if (fixed) {
            const alt = run(fixed);
            // like the server: the corrected results replace little or no-author results
            if ((found(r) < 3 && found(alt) > found(r)) || (wantsNames && names(r) === 0 && names(alt) > 0)) { r = alt; corrected = fixed; }
            else if (found(alt) > found(r)) didYouMean = fixed;
          }
        }
        const authors = r.authors.map((a) => ({ id: a.id, name: a.name, count: a.bookCount }));
        const seriesRes = r.series.map((s) => {
              const bookIds = lib.booksBySeries.get(s.id) ?? [];
              const authorNames = [...new Set(bookIds.flatMap((id) => lib.bookById.get(id)!.authorIds.map((aid) => lib.authors[aid - 1].name)))];
              return { id: s.id, name: s.name, count: s.bookCount, authors: authorNames.slice(0, 3).join(', ') || 'various authors' };
            });
        let books = r.books;
        const genre = url.searchParams.get('genre');
        const langF = url.searchParams.get('lang');
        const extF = url.searchParams.get('ext');
        const from = url.searchParams.get('from');
        const to = url.searchParams.get('to');
        const facetGenre = new Map<number, number>(), facetLang = new Map<string, number>(), facetExt = new Map<string, number>();
        for (const b of books) {
          for (const g of b.genreIds) facetGenre.set(g, (facetGenre.get(g) ?? 0) + 1);
          facetLang.set(b.lang, (facetLang.get(b.lang) ?? 0) + 1);
          facetExt.set(b.ext, (facetExt.get(b.ext) ?? 0) + 1);
        }
        if (genre) { const ids = genre.split(',').map(Number); books = books.filter((b) => b.genreIds.some((g) => ids.includes(g))); }
        if (langF) { const ls = langF.split(','); books = books.filter((b) => ls.includes(b.lang)); }
        if (extF) { const es = extF.split(','); books = books.filter((b) => es.includes(b.ext)); }
        if (from) books = books.filter((b) => b.date >= from);
        if (to) books = books.filter((b) => b.date <= to);
        const ratedBooks = rateFilterSort(lib, books, url.searchParams);
        if (typeof ratedBooks === 'string') return fail(res, 400, 'bad_request', ratedBooks);
        books = ratedBooks;
        const limit = Math.min(Number(url.searchParams.get('limit') ?? 200) || 200, 1000);
        const rows = url.searchParams.get('group') === '1'
          ? (() => {
              const groups = groupBooks(books);
              const s = url.searchParams.get('sort');
              if (s === 'my' || s === 'lib' || s === 'ext') {
                const key = (b: MockBook) => s === 'my' ? bookRating(lib.id, b.id) : s === 'lib' ? libRating(b)
                  : (extRating(b)?.avg ?? 0) * 1e6 + (extRating(b)?.votes ?? 0);
                groups.sort((x, y) => key(y.best) - key(x.best));
              }
              return groups.map((g) => toRow(lib, g));
            })()
          : books.map((b) => toBook(lib, b));
        const total = rows.length;
        const shown = rows.slice(0, limit);
        const highlight = highlightWords(r.tokens, [
          ...authors.map((a) => a.name), ...seriesRes.map((s) => s.name),
          ...shown.flatMap((b) => [b.title, ...b.authors.map((a) => a.name), b.series?.name ?? '']),
        ]);
        return send(res, 200, {
          tookMs: Date.now() - t0,
          authors, series: seriesRes, books: shown, total,
          facets: {
            genre: [...facetGenre.entries()], lang: [...facetLang.entries()], ext: [...facetExt.entries()],
          },
          corrected, didYouMean, highlight,
        });
      }
      if (path === '/api/v1/languages' && method === 'GET') {
        const lib = catalog(Number(url.searchParams.get('lib') ?? 1));
        const counts = new Map<string, number>();
        for (const b of lib.books) counts.set(b.lang, (counts.get(b.lang) ?? 0) + 1);
        return send(res, 200, [...counts.entries()]);
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/books\/(\d+)\/rating$/);
      if (m && method === 'PUT') {
        const body = await readBody(req);
        store.ratings.set(`${m[1]}:${m[2]}`, Number(body.rating) || 0);
        return send(res, 204);
      }

      // ---- Shelves --------------------------------------------------
      if (path === '/api/v1/shelves' && method === 'GET') {
        for (const s of store.shelves) s.count = store.shelfBooks.get(s.id)?.size ?? 0;
        return send(res, 200, store.shelves);
      }
      if (path === '/api/v1/shelves' && method === 'POST') {
        const body = await readBody(req);
        const shelf = { id: store.nextShelfId++, name: body.name, color: body.color, count: 0 };
        store.shelves.push(shelf);
        store.shelfBooks.set(shelf.id, new Set());
        return send(res, 200, shelf);
      }
      m = matchLib(req, /^\/api\/v1\/shelves\/(\d+)$/);
      if (m && method === 'PATCH') {
        const shelf = store.shelves.find((s) => s.id === Number(m![1]));
        if (!shelf) return fail(res, 404, 'not_found', 'Shelf not found');
        Object.assign(shelf, await readBody(req));
        return send(res, 200, shelf);
      }
      if (m && method === 'DELETE') {
        store.shelves = store.shelves.filter((s) => s.id !== Number(m![1]));
        store.shelfBooks.delete(Number(m[1]));
        return send(res, 204);
      }
      m = matchLib(req, /^\/api\/v1\/shelves\/(\d+)\/books$/);
      if (m && method === 'POST') {
        const body = await readBody(req);
        const set = store.shelfBooks.get(Number(m![1])) ?? new Set();
        const lib = catalog(Number(body.library));
        for (const id of body.books as number[]) {
          const b = lib.bookById.get(id);
          if (!b) continue;
          const key = `${lib.id}:${b.key}`;
          if (body.add) set.add(key); else set.delete(key);
        }
        store.shelfBooks.set(Number(m[1]), set);
        const shelf = store.shelves.find((s) => s.id === Number(m![1]))!;
        shelf.count = set.size;
        return send(res, 200, shelf);
      }

      // ---- Devices / sending -----------------------------------------
      if (path === '/api/v1/devices' && method === 'GET') return send(res, 200, store.devices);
      if (path === '/api/v1/devices' && method === 'POST') {
        const body = await readBody(req);
        const device = { ...body, id: store.nextDeviceId++ };
        store.devices.push(device);
        return send(res, 200, device);
      }
      if (path === '/api/v1/devices/order' && method === 'PUT') {
        const body = await readBody(req);
        const ids: number[] = body.ids ?? [];
        if (ids.some((id) => !store.devices.some((d) => d.id === id))) return fail(res, 404, 'not_found', 'device not found');
        store.devices = [...ids.map((id) => store.devices.find((d) => d.id === id)!), ...store.devices.filter((d) => !ids.includes(d.id))];
        return send(res, 200, store.devices);
      }
      m = matchLib(req, /^\/api\/v1\/devices\/(\d+)$/);
      if (m && method === 'PUT') {
        const idx = store.devices.findIndex((d) => d.id === Number(m![1]));
        if (idx < 0) return fail(res, 404, 'not_found', 'Device not found');
        const body = await readBody(req);
        store.devices[idx] = { ...body, id: Number(m[1]) };
        return send(res, 200, store.devices[idx]);
      }
      if (m && method === 'DELETE') {
        store.devices = store.devices.filter((d) => d.id !== Number(m![1]));
        return send(res, 204);
      }
      if (path === '/api/v1/send' && method === 'POST') {
        const body = await readBody(req);
        const device = store.devices.find((d) => d.id === body.device);
        if (!device) return fail(res, 404, 'not_found', 'Device not found');
        if (device.kind === 'email') {
          // same rule as the server: smtp.allowedRecipients, `*` = any characters
          const to = String(body.target || device.target || '').trim().toLowerCase();
          const ok = store.settings.smtp.allowedRecipients.some((p) =>
            new RegExp('^' + p.toLowerCase().split('*').map((x) => x.replace(/[.+?^${}()|[\]\\]/g, '\\$&')).join('.*') + '$').test(to));
          if (!ok) return fail(res, 403, 'forbidden', `${to} is not an allowed recipient`);
        }
        const kind = device.kind === 'email' ? 'send' : device.kind === 'folder' ? 'export' : 'download';
        // "send whole series": the series' live books in reading order (like the server)
        const lib = catalog(Number(body.library) || 1);
        const ids: number[] = [...(body.books ?? [])];
        let seriesName = '';
        for (const sid of (body.series ?? []) as number[]) {
          const s = lib.series[sid - 1];
          if (!s) return fail(res, 404, 'not_found', 'no books in this series');
          seriesName ||= s.name;
          for (const id of lib.booksBySeries.get(sid) ?? []) if (!lib.bookById.get(id)?.deleted && !ids.includes(id)) ids.push(id);
        }
        if (!ids.length) return fail(res, 400, 'bad_request', 'no books selected');
        recordHistory(Number(body.library), ids);
        const items: JobItem[] = ids.slice(0, 200).map((id) => ({
          bookId: id, title: lib.bookById.get(id)?.title ?? `#${id}`, state: 'queued', detail: '', attempts: 0, size: null, mail: null,
        }));
        const n = ids.length;
        const what = `${seriesName ? `«${seriesName}» · ` : ''}${n} book${n === 1 ? '' : 's'}`;
        const job = createJob(kind, `${kind === 'send' ? 'Send to' : kind === 'export' ? 'Export to' : 'Download for'} ${device.name} · ${what}`, items);
        const target = String(body.target || device.target || '');
        store.jobRequests.set(job.id, { target, device });
        runSendJob(job, { email: device.kind === 'email', kindle: /@(free\.)?kindle\.com$/i.test(target) });
        return send(res, 200, job);
      }
      if (path === '/api/v1/handoff' && method === 'POST') {
        const body = await readBody(req);
        const lib = catalog(Number(body.library) || 1);
        const b = lib.bookById.get(Number(body.book));
        if (!b) return fail(res, 404, 'not_found', 'book not found');
        const device = store.devices.find((d) => d.id === body.device) ?? store.devices.find((d) => d.preset === 'apple-books');
        const format = body.format ?? (device && device.kind !== 'email' ? device.format : 'epub');
        const token = Array.from({ length: 22 }, (_, i) => 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnpqrstuvwxyz23456789'[(b.id * 31 + i * 17 + store.handoffs.size * 7) % 56]).join('');
        const expires = Date.now() + 15 * 60_000;
        store.handoffs.set(token, { lib: lib.id, book: b.id, format, uses: 0, expires });
        const host = req.headers.host ?? 'localhost:5173';
        return send(res, 200, {
          url: `/h/${token}`, absoluteUrl: `http://${host}/h/${token}`, expiresAt: new Date(expires).toISOString(),
          maxUses: 3, format, fileName: `${b.title}.${format === 'kepub' ? 'kepub.epub' : format}`, title: b.title,
        });
      }
      m = matchLib(req, /^\/api\/v1\/jobs\/([\w-]+)\/retry$/);
      if (m && method === 'POST') {
        const job = store.jobs.find((j) => j.id === m![1]);
        if (!job) return fail(res, 404, 'not_found', 'job not found');
        if (!job.retryable) return fail(res, 409, 'conflict', 'this job cannot be retried');
        const r = store.jobRequests.get(job.id);
        job.state = 'queued'; job.finishedAt = null; job.hint = null; job.retryable = false; job.message = '';
        const redo = (job.items ?? []).filter((it) => it.state === 'failed');
        for (const it of redo) { it.state = 'queued'; it.detail = ''; it.title = it.title.replace(' (fail)', ''); }
        const keep = (job.items ?? []).filter((it) => it.state !== 'queued');
        const sub = { ...job, items: redo };
        runSendJob(sub as typeof job, { email: r?.device.kind === 'email', kindle: /@(free\.)?kindle\.com$/i.test(r?.target ?? '') });
        // mirror the sub-run into the job
        const mirror = setInterval(() => {
          job.items = [...keep, ...redo]; job.state = sub.state; job.message = sub.message; job.progress = sub.progress;
          job.hint = sub.hint; job.retryable = sub.retryable; job.finishedAt = sub.finishedAt;
          broadcast('job', job);
          if (sub.state === 'done' || sub.state === 'failed') clearInterval(mirror);
        }, 200);
        return send(res, 200, job);
      }
      if (path === '/api/v1/fonts' && method === 'GET') {
        return send(res, 200, ['Literata', 'IBM Plex Sans', 'PT Serif', 'Georgia', 'Verdana']);
      }

      // ---- Jobs -----------------------------------------------------
      if (path === '/api/v1/jobs' && method === 'GET') return send(res, 200, store.jobs.slice(0, 50));
      m = matchLib(req, /^\/api\/v1\/jobs\/([\w-]+)\/cancel$/);
      if (m && method === 'POST') {
        const job = store.jobs.find((j) => j.id === m![1]);
        if (!job) return fail(res, 404, 'not_found', 'Job not found');
        job.state = 'cancelled'; job.finishedAt = new Date().toISOString();
        broadcast('job', job);
        return send(res, 200, job);
      }
      if (path === '/api/v1/jobs' && method === 'DELETE' && url.searchParams.get('finished') === '1') {
        store.jobs = store.jobs.filter((j) => j.state === 'running' || j.state === 'queued');
        return send(res, 204);
      }
      m = matchLib(req, /^\/api\/v1\/jobs\/([\w-]+)\/download$/);
      if (m && method === 'GET') {
        res.setHeader('Content-Type', 'application/octet-stream');
        res.setHeader('Content-Disposition', 'attachment; filename="books.zip"');
        res.end('Mock download content');
        return;
      }
      if (path === '/api/v1/events' && method === 'GET') {
        res.writeHead(200, {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          Connection: 'keep-alive',
        });
        const listener = (event: string, data: unknown) => {
          res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
        };
        store.sseClients.add(listener);
        const heartbeat = setInterval(() => res.write(': ping\n\n'), 25000);
        req.on('close', () => { clearInterval(heartbeat); store.sseClients.delete(listener); });
        return;
      }

      // ---- API tokens (MCP) ----------------------------------------------
      if (path === '/api/v1/me/tokens' && method === 'GET') {
        return send(res, 200, {
          tokens: store.tokens, scopes: ['read', 'write', 'send'],
          mcp: { enabled: store.settings.mcp?.enabled ?? true, url: `http://${req.headers.host ?? 'localhost'}/mcp`, oauth: true },
        });
      }
      if (path === '/api/v1/me/tokens' && method === 'POST') {
        const body = await readBody(req);
        const scopes = ['read', 'write', 'send'].filter((s) => (body.scopes ?? []).includes(s));
        if (!String(body.name ?? '').trim()) return fail(res, 400, 'bad_request', 'token name must have 1..100 characters');
        if (!scopes.length) return fail(res, 400, 'bad_request', 'choose at least one scope');
        let secret = 'fl_';
        const abc = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
        for (let i = 0; i < 43; i++) secret += abc[Math.floor(Math.random() * abc.length)];
        const days = Number(body.expiresInDays) || 0;
        const token = {
          id: store.nextTokenId++, name: String(body.name).trim(), prefix: secret.slice(0, 11), scopes,
          createdAt: new Date().toISOString(), lastUsedAt: null,
          expiresAt: days ? new Date(Date.now() + days * 86400000).toISOString() : null,
        };
        store.tokens.unshift(token);
        return send(res, 200, { token, secret });
      }
      if (path === '/api/v1/me/tokens/audit' && method === 'GET') return send(res, 200, store.audit);
      m = matchLib(req, /^\/api\/v1\/me\/tokens\/(\d+)$/);
      if (m && method === 'DELETE') {
        const before = store.tokens.length;
        store.tokens = store.tokens.filter((x) => x.id !== Number(m![1]));
        return before === store.tokens.length ? fail(res, 404, 'not_found', 'token not found') : send(res, 204);
      }

      // ---- OAuth (consent page, authorized apps) ------------------------
      if (path === '/api/v1/me/oauth/apps' && method === 'GET') return send(res, 200, store.oauthApps);
      m = matchLib(req, /^\/api\/v1\/me\/oauth\/apps\/(\d+)$/);
      if (m && method === 'DELETE') {
        const before = store.oauthApps.length;
        store.oauthApps = store.oauthApps.filter((x) => x.id !== Number(m![1]));
        return before === store.oauthApps.length ? fail(res, 404, 'not_found', 'app not found') : send(res, 204);
      }
      m = matchLib(req, /^\/api\/v1\/oauth\/requests\/([\w-]+)$/);
      if (m) {
        const r = store.oauthRequests[m[1]];
        if (!r) return fail(res, 404, 'not_found', 'this authorization request is unknown or expired; start the connection again in the app');
        if (method === 'GET') return send(res, 200, r);
        if (method === 'POST') {
          const body = await readBody(req);
          if (body.csrf !== r.csrf) return fail(res, 403, 'forbidden', 'invalid request token');
          const u = new URL(r.redirectUri);
          if (body.approve) {
            const scopes = (body.scopes ?? []).filter((s: string) => r.scopes.includes(s as never));
            if (!scopes.length) return fail(res, 400, 'bad_request', 'choose at least one permission');
            u.searchParams.set('code', 'mock-code');
          } else {
            u.searchParams.set('error', 'access_denied');
          }
          u.searchParams.set('state', 'mock-state');
          u.searchParams.set('iss', `http://${req.headers.host ?? 'localhost'}`);
          return send(res, 200, { redirect: u.toString() });
        }
      }

      // ---- Settings / users -------------------------------------------
      if (path === '/api/v1/settings' && method === 'GET') {
        const total = store.libraries.reduce((n, l) => n + l.bookCount, 0);
        return send(res, 200, {
          ...store.settings,
          externalRatings: {
            enabled: store.settings.externalRatings?.enabled ?? true, source: 'openlibrary', contactSet: false,
            progress: { lookedUp: Math.round(total * 0.42), found: Math.round(total * 0.3), rated: Math.round(total * 0.24), total },
            queued: 3, requests: 12840, pausedFor: 0, lastError: null,
          },
          mcp: { enabled: store.settings.mcp?.enabled ?? true, url: null },
        });
      }
      if (path === '/api/v1/settings' && method === 'PUT') {
        const body = await readBody(req);
        store.settings = {
          ...body,
          externalRatings: { enabled: body.externalRatings?.enabled ?? store.settings.externalRatings?.enabled ?? true },
          mcp: { enabled: body.mcp?.enabled ?? store.settings.mcp?.enabled ?? true },
          smtp: (({ password: _pw, ...rest }) => rest)({ ...store.settings.smtp, ...body.smtp, passwordSet: body.smtp?.password === undefined ? store.settings.smtp.passwordSet : body.smtp.password !== '' }),
        };
        return send(res, 200, store.settings);
      }
      if (path === '/api/v1/settings/smtp/test' && method === 'POST') return send(res, 204);
      if (path === '/api/v1/users' && method === 'GET') {
        return send(res, 200, store.users.map((u) => ({ ...u, hasPassword: true, sso: null })));
      }
      if (path === '/api/v1/users' && method === 'POST') {
        const body = await readBody(req);
        const user = { id: store.users.length + 1, username: body.username, role: body.role };
        store.users.push(user);
        return send(res, 200, user);
      }
      m = matchLib(req, /^\/api\/v1\/users\/(\d+)$/);
      if (m && method === 'PATCH') {
        const user = store.users.find((u) => u.id === Number(m![1]));
        if (!user) return fail(res, 404, 'not_found', 'User not found');
        const body = await readBody(req);
        if (body.role) user.role = body.role;
        return send(res, 200, user);
      }
      if (m && method === 'DELETE') {
        store.users = store.users.filter((u) => u.id !== Number(m![1]));
        return send(res, 204);
      }
      if (path === '/api/v1/me/prefs') {
        // Prefs are per browser (cookie), so parallel Playwright tests don't share
        // pane widths, sort orders etc. through the one mock "admin" user.
        let client = /(?:^|;\s*)freelib_mock_client=([\w-]+)/.exec(req.headers.cookie ?? '')?.[1];
        if (!client) {
          client = `c${store.nextClientId++}`;
          res.setHeader('Set-Cookie', `freelib_mock_client=${client}; Path=/; SameSite=Lax`);
        }
        if (method === 'PUT') store.prefs.set(client, await readBody(req));
        return send(res, 200, store.prefs.get(client) ?? {});
      }

      if (path.startsWith('/opds')) {
        res.setHeader('Content-Type', 'application/atom+xml; charset=utf-8');
        res.end('<?xml version="1.0" encoding="UTF-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>freeLib (mock OPDS)</title></feed>');
        return;
      }

      return fail(res, 404, 'not_found', `No mock handler for ${method} ${path}`);
    } catch (err) {
      console.error('[mock api]', err);
      return fail(res, 500, 'internal', String(err));
    }
  });
}
