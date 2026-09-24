import type { Connect } from 'vite';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { store, catalog, createJob, runJobProgress, broadcast } from './store';
import { placeholderCover } from './covers';
import { normalize } from './normalize';
import type { Book, BookDetail, AuthorRef, SeriesRef } from '../src/lib/api/types';
import type { MockBook, MockLibrary } from './gen';

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

function toBook(lib: MockLibrary, b: MockBook): Book {
  return {
    id: b.id, key: b.key, title: b.title,
    authors: toAuthorRefs(lib, b.authorIds),
    series: toSeriesRef(lib, b.seriesId), serno: b.serno,
    genres: b.genreIds, lang: b.lang, ext: b.ext, size: b.size, date: b.date, deleted: b.deleted,
    rating: bookRating(lib.id, b.id), shelves: bookShelves(lib.id, b.key),
  };
}

function toDetail(lib: MockLibrary, b: MockBook): BookDetail {
  return {
    ...toBook(lib, b),
    annotation: `<p>Аннотация к книге «${b.title}» появится здесь после первого открытия файла и будет закэширована на сервере.</p>`,
    hasCover: true,
    file: `${b.key.split(':')[0]}-archive.zip / ${b.id}.${b.ext}`,
    keywords: '',
    formats: ['original', 'epub', b.ext !== 'epub' ? 'epub' : 'fb2', 'kepub', 'azw3'].filter((v, i, a) => a.indexOf(v) === i),
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

export function installMockApi(server: Connect.Server) {
  server.use(async (req, res, next) => {
    const url = new URL(req.url ?? '/', 'http://localhost');
    const path = url.pathname;
    const method = req.method ?? 'GET';
    if (!path.startsWith('/api/v1') && !path.startsWith('/opds')) return next();

    // simulate small network latency for realism
    await new Promise((r) => setTimeout(r, 15 + Math.random() * 25));

    try {
      // ---- Session --------------------------------------------------
      if (path === '/api/v1/session' && method === 'GET') {
        return send(res, 200, { user: { id: 1, username: 'admin', role: 'admin' }, openMode: true });
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
              lib.bookCount = c.books.length; lib.authorCount = c.authors.length; lib.seriesCount = c.series.length;
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
        return send(res, 200, {
          version: 1, columns: ['id', 'name', 'count'],
          rows: lib.authors.map((a) => [a.id, a.name, a.bookCount]),
          letters: lib.authorLetters,
        }, { 'Cache-Control': url.searchParams.has('v') ? 'public, max-age=31536000, immutable' : 'no-cache' });
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/series$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        return send(res, 200, {
          version: 1, columns: ['id', 'name', 'count'],
          rows: lib.series.map((s) => [s.id, s.name, s.bookCount]),
          letters: lib.seriesLetters,
        });
      }
      m = matchLib(req, /^\/api\/v1\/libraries\/(\d+)\/genres$/);
      if (m && method === 'GET') {
        const lib = catalog(Number(m[1]));
        return send(res, 200, lib.genres);
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
        if (lang) items = items.filter((b) => b.lang === lang);
        if (ext) items = items.filter((b) => b.ext === ext);
        if (!showDeleted) items = items.filter((b) => !b.deleted);

        const { page, next } = paginate(items, cursor, limit);
        return send(res, 200, { books: page.map((b) => toBook(lib, b)), nextCursor: next, total: items.length });
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
        const q = normalize(q0);
        const words = q.split(' ').filter(Boolean);
        const kind = url.searchParams.get('kind') ?? 'all';
        const t0 = Date.now();
        const matchWords = (hay: string) => words.every((w) => hay.split(' ').some((tok) => tok.startsWith(w)));

        const authors = kind === 'all' || kind === 'authors'
          ? lib.authors.filter((a) => matchWords(a.sortKey)).slice(0, 20).map((a) => ({ id: a.id, name: a.name, count: a.bookCount }))
          : [];
        const seriesRes = kind === 'all' || kind === 'series'
          ? lib.series.filter((s) => matchWords(s.sortKey)).slice(0, 20).map((s) => {
              const bookIds = lib.booksBySeries.get(s.id) ?? [];
              const authorNames = [...new Set(bookIds.flatMap((id) => lib.bookById.get(id)!.authorIds.map((aid) => lib.authors[aid - 1].name)))];
              return { id: s.id, name: s.name, count: s.bookCount, authors: authorNames.slice(0, 3).join(', ') || 'разные авторы' };
            })
          : [];
        let books = kind === 'all' || kind === 'books' ? lib.books.filter((b) => matchWords(b.sortKey)) : [];
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
        const limit = Math.min(Number(url.searchParams.get('limit') ?? 200) || 200, 1000);
        const total = books.length;
        books = books.slice(0, limit);
        return send(res, 200, {
          tookMs: Date.now() - t0,
          authors, series: seriesRes, books: books.map((b) => toBook(lib, b)), total,
          facets: {
            genre: [...facetGenre.entries()], lang: [...facetLang.entries()], ext: [...facetExt.entries()],
          },
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
        const kind = device.kind === 'email' ? 'send' : device.kind === 'folder' ? 'export' : 'download';
        const job = createJob(kind, `${kind === 'send' ? 'Send to' : kind === 'export' ? 'Export to' : 'Download for'} ${device.name} · ${body.books.length} books`);
        runJobProgress(job);
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

      // ---- Settings / users -------------------------------------------
      if (path === '/api/v1/settings' && method === 'GET') return send(res, 200, store.settings);
      if (path === '/api/v1/settings' && method === 'PUT') {
        const body = await readBody(req);
        store.settings = {
          ...body,
          smtp: { ...store.settings.smtp, ...body.smtp, passwordSet: body.smtp?.password ? true : store.settings.smtp.passwordSet },
        };
        return send(res, 200, store.settings);
      }
      if (path === '/api/v1/settings/smtp/test' && method === 'POST') return send(res, 204);
      if (path === '/api/v1/users' && method === 'GET') return send(res, 200, store.users);
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
      if (path === '/api/v1/me/prefs' && method === 'GET') return send(res, 200, store.prefs);
      if (path === '/api/v1/me/prefs' && method === 'PUT') {
        store.prefs = await readBody(req);
        return send(res, 200, store.prefs);
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
