import { generateLibrary, type MockLibrary } from './gen';
import type { Device, Job, Shelf, Settings, User, Library, ConvertOptions, ApiToken, AuditRow } from '../src/lib/api/types';

const defaultOptions: ConvertOptions = {
  hyphenate: 'soft', footnotes: 'end', dropCaps: false, breakAfterChapter: true,
  tocPlacement: 'start', createCover: 'missing', coverLabel: '%s %n', joinSeries: false,
  transliterate: false, annotation: true, fontFamily: null, userCss: null,
};

export type LibraryMeta = Library;

export const store = {
  catalogs: new Map<number, MockLibrary>(),
  libraries: [] as LibraryMeta[],
  users: [{ id: 1, username: 'admin', role: 'admin' as const }] as User[],
  devices: [
    { id: 1, name: 'Kindle', kind: 'email', format: 'epub', target: 'reader@kindle.com', fileName: '%a - %s %n - %b', shared: true, options: defaultOptions },
    { id: 2, name: 'Kindle (USB)', kind: 'download', format: 'azw3', target: null, fileName: '%a - %s %n - %b', shared: true, options: defaultOptions },
    { id: 3, name: 'Apple Books', kind: 'download', format: 'epub', target: null, fileName: '%a - %s %n - %b', shared: true, options: defaultOptions },
    { id: 4, name: 'Kobo', kind: 'download', format: 'kepub', target: null, fileName: '%a - %s %n - %b', shared: true, options: defaultOptions },
    { id: 5, name: 'Server folder', kind: 'folder', format: 'epub', target: 'incoming', fileName: '%a - %s %n - %b', shared: true, options: defaultOptions },
    { id: 6, name: 'Original', kind: 'download', format: 'original', target: null, fileName: '%a - %s %n - %b', shared: true, options: defaultOptions },
  ] as Device[],
  shelves: [
    { id: 1, name: 'To read', color: '#1F5F5B', count: 0 },
    { id: 2, name: 'Favourites', color: '#B8741A', count: 0 },
    { id: 3, name: 'For kids', color: '#6B4E8A', count: 0 },
  ] as Shelf[],
  shelfBooks: new Map<number, Set<string>>([[1, new Set()], [2, new Set()], [3, new Set()]]),
  ratings: new Map<string, number>(), // `${lib}:${bookId}` -> rating
  jobs: [] as Job[],
  settings: {
    smtp: { host: 'smtp.example.com', port: 587, security: 'starttls', username: 'freelib', from: 'freelib@example.com', passwordSet: true, pauseSeconds: 2, allowedRecipients: ['*@kindle.com', '*@free.kindle.com'], dailyLimitPerUser: 100, subject: '%b' },
    opds: { enabled: true, requireAuth: true },
    calibre: { available: true, version: '7.4.0' },
  } as Settings,
  prefs: new Map<string, Record<string, unknown>>(),
  tokens: [
    { id: 1, name: 'Claude Desktop', prefix: 'fl_Q3v9KmZ2', scopes: ['read', 'send'], createdAt: '2026-09-10T08:12:00Z', lastUsedAt: '2026-09-24T19:40:00Z', expiresAt: null },
  ] as ApiToken[],
  nextTokenId: 2,
  audit: [
    { id: 3, tokenId: 1, tokenName: 'Claude Desktop', tool: 'send_books', ok: true, detail: '{"book_ids":[5],"device":"default"}', at: '2026-09-24T19:40:00Z' },
    { id: 2, tokenId: 1, tokenName: 'Claude Desktop', tool: 'suggest_candidates', ok: true, detail: '{"limit":10}', at: '2026-09-24T19:38:10Z' },
    { id: 1, tokenId: 1, tokenName: 'Claude Desktop', tool: 'rate_book', ok: false, detail: '{"id":5,"rating":5}', at: '2026-09-24T19:37:02Z' },
  ] as AuditRow[],
  nextClientId: 1,
  nextJobId: 1,
  nextDeviceId: 7,
  nextShelfId: 4,
  nextLibraryId: 3,
  sseClients: new Set<(event: string, data: unknown) => void>(),
};

export function catalog(libId: number): MockLibrary {
  let c = store.catalogs.get(libId);
  if (!c) {
    c = generateLibrary(libId, 1000 + libId);
    store.catalogs.set(libId, c);
  }
  return c;
}

function libMetaFor(id: number, name: string, isDefault: boolean): LibraryMeta {
  const c = catalog(id);
  return {
    id, name, path: `/books/${name}`, inpx: `${name}.inpx`,
    firstAuthorOnly: false, skipDeleted: false, isDefault,
    bookCount: c.books.filter((b) => !b.deleted).length, authorCount: c.authorRows.length, seriesCount: c.seriesRows.length,
    importedAt: '2026-08-01T10:00:00Z', catalogVersion: 1, newSinceLastVisit: 12,
    status: { state: 'idle' },
    opdsUrl: `/opds/${id}`,
    externalRatings: { lookedUp: Math.round(c.books.length * 0.42), found: Math.round(c.books.length * 0.3), rated: Math.round(c.books.length * 0.24) },
  };
}

if (store.libraries.length === 0) {
  store.libraries.push(libMetaFor(1, 'Flibusta', true));
  store.libraries.push(libMetaFor(2, 'Home Collection', false));
}

export function broadcast(event: string, data: unknown) {
  for (const send of store.sseClients) send(event, data);
}

export function createJob(kind: Job['kind'], title: string): Job {
  const job: Job = {
    id: String(store.nextJobId++), kind, title, state: 'queued', progress: 0, message: '',
    log: [], downloadUrl: null, createdAt: new Date().toISOString(), finishedAt: null,
  };
  store.jobs.unshift(job);
  broadcast('job', job);
  return job;
}

export function runJobProgress(job: Job, opts?: { steps?: number; intervalMs?: number; onDone?: () => void }) {
  const steps = opts?.steps ?? 8;
  const interval = opts?.intervalMs ?? 350;
  job.state = 'running';
  broadcast('job', job);
  let i = 0;
  const timer = setInterval(() => {
    i++;
    job.progress = Math.min(1, i / steps);
    job.message = `Step ${i}/${steps}`;
    if (i >= steps) {
      clearInterval(timer);
      job.state = 'done';
      job.progress = 1;
      job.message = 'Done';
      job.finishedAt = new Date().toISOString();
      if (job.kind === 'download' || job.kind === 'export') job.downloadUrl = `/api/v1/jobs/${job.id}/download`;
      opts?.onDone?.();
    }
    broadcast('job', job);
  }, interval);
}
