import { generateLibrary, type MockLibrary } from './gen';
import type { Device, Job, JobItem, Shelf, Settings, User, Library, ConvertOptions, ApiToken, AuditRow, OAuthApp, OAuthRequest } from '../src/lib/api/types';

// the server's tuned presets (server/crates/server/src/presets.rs, docs/web/DEVICES.md)
const defaultOptions: ConvertOptions = {
  hyphenate: 'soft', footnotes: 'popup', dropCaps: false, breakAfterChapter: true,
  tocPlacement: 'end', createCover: 'missing', coverLabel: null, joinSeries: false,
  transliterate: false, annotation: true, fontFamily: null, userCss: null,
};
const fullHyphens: ConvertOptions = { ...defaultOptions, hyphenate: 'full' };
const endNotes: ConvertOptions = { ...defaultOptions, footnotes: 'end' };

export type LibraryMeta = Library;

export const store = {
  catalogs: new Map<number, MockLibrary>(),
  libraries: [] as LibraryMeta[],
  users: [{ id: 1, username: 'admin', role: 'admin' as const }] as User[],
  devices: [
    { id: 1, name: 'Kindle', kind: 'email', format: 'epub', target: 'reader@kindle.com', fileName: '%a - %s %n - %b', shared: true, options: defaultOptions, preset: 'kindle-email' },
    { id: 2, name: 'Kindle (USB)', kind: 'download', format: 'azw3', target: null, fileName: '%a - %s %n - %b', shared: true, options: defaultOptions, preset: 'kindle-usb' },
    { id: 3, name: 'Apple Books', kind: 'download', format: 'epub', target: null, fileName: '%a - %s %n - %b', shared: true, options: fullHyphens, preset: 'apple-books' },
    { id: 4, name: 'Kobo', kind: 'download', format: 'kepub', target: null, fileName: '%a - %s %n - %b', shared: true, options: fullHyphens, preset: 'kobo' },
    { id: 5, name: 'Server folder', kind: 'folder', format: 'epub', target: 'incoming', fileName: '%a - %s %n - %b', shared: true, options: endNotes, preset: 'server-folder' },
    { id: 6, name: 'Original', kind: 'download', format: 'original', target: null, fileName: '%a - %s %n - %b', shared: true, options: defaultOptions, preset: 'original' },
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
    smtp: { host: 'smtp.example.com', port: 587, security: 'starttls', username: 'freelib', from: 'freelib@example.com', passwordSet: true, pauseSeconds: 2, allowedRecipients: ['*@kindle.com', '*@free.kindle.com'], dailyLimitPerUser: 100, subject: '%b', maxAttachments: 25, maxMailMb: 50, retries: 3, retryDelaySeconds: 30 },
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
    { id: 4, tokenId: null, tokenName: null, grantId: 1, appName: 'Claude', tool: 'search_books', ok: true, detail: '{"query":"Стругацкие"}', at: '2026-09-25T10:02:00Z' },
  ] as AuditRow[],
  oauthApps: [
    { id: 1, clientName: 'Claude', clientKind: 'cimd', verifiedHost: 'claude.ai', redirectHost: 'claude.ai', scopes: ['read', 'write', 'send'], createdAt: '2026-09-20T18:00:00Z', lastUsedAt: '2026-09-25T10:02:00Z' },
    { id: 2, clientName: 'Claude Code', clientKind: 'cimd', verifiedHost: 'claude.ai', redirectHost: 'localhost', scopes: ['read'], createdAt: '2026-09-22T09:30:00Z', lastUsedAt: null },
    { id: 3, clientName: 'My MCP script', clientKind: 'dcr', verifiedHost: null, redirectHost: '127.0.0.1', scopes: ['read', 'write'], createdAt: '2026-09-23T12:00:00Z', lastUsedAt: '2026-09-23T12:05:00Z' },
  ] as OAuthApp[],
  /** pending consent requests by id (`claude`, `local`; anything else is unknown) */
  oauthRequests: {
    claude: {
      client: { name: 'Claude', kind: 'cimd', verifiedHost: 'claude.ai', clientUri: 'https://claude.ai/oauth/mcp-oauth-client-metadata' },
      redirectUri: 'https://claude.ai/api/mcp/auth_callback', redirectHost: 'claude.ai', loopback: false,
      scopes: ['read', 'write', 'send'], resource: 'http://localhost/mcp', csrf: 'mock-csrf',
    },
    local: {
      client: { name: 'My MCP script', kind: 'dcr', verifiedHost: null, clientUri: null },
      redirectUri: 'http://127.0.0.1:43210/callback', redirectHost: '127.0.0.1', loopback: true,
      scopes: ['read', 'write'], resource: 'http://localhost/mcp', csrf: 'mock-csrf',
    },
  } as Record<string, OAuthRequest>,
  nextClientId: 1,
  /** phone hand-off links: token → book */
  handoffs: new Map<string, { lib: number; book: number; format: string; uses: number; expires: number }>(),
  /** e-mail send requests kept for POST /jobs/:id/retry */
  jobRequests: new Map<string, { target: string; device: Device }>(),
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

export function createJob(kind: Job['kind'], title: string, items: JobItem[] = []): Job {
  const job: Job = {
    id: String(store.nextJobId++), kind, title, state: 'queued', progress: 0, message: '',
    log: [], downloadUrl: null, createdAt: new Date().toISOString(), finishedAt: null,
    items, retryable: false, hint: null,
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

/**
 * Like the server's e-mail jobs: every book is converted, then the books travel in e-mails of
 * up to 25 (`converting → converted → sending → accepted` with the mail server's reply). A book
 * titled with "(fail)" is refused permanently (a 554 reply) so failure paths can be tested.
 */
export function runSendJob(job: Job, opts: { email: boolean; kindle: boolean; stepMs?: number }) {
  const step = opts.stepMs ?? 250;
  const items = job.items ?? [];
  const ticks: Array<() => void> = [];
  job.state = 'running';
  for (const it of items) {
    ticks.push(() => { it.state = 'converting'; job.message = `Converting ${it.title}`; });
    ticks.push(() => { it.state = opts.email ? 'converted' : 'ready'; it.size = 180_000 + (it.bookId % 50) * 7_000; });
  }
  if (opts.email) {
    const mails: JobItem[][] = [];
    items.forEach((it, i) => { (mails[Math.floor(i / 25)] ??= []).push(it); });
    mails.forEach((mail, m) => {
      ticks.push(() => {
        job.message = `Sending e-mail ${m + 1} of ${mails.length}`;
        for (const it of mail) { it.state = 'sending'; it.attempts += 1; it.mail = m + 1; it.detail = 'handed to smtp.example.com'; }
      });
      ticks.push(() => {
        const qid = `4F${(1000 + job.id.length * 97 + m * 13).toString(16).toUpperCase()}A${m}`;
        for (const it of mail) {
          if (/\(fail\)/.test(it.title)) { it.state = 'failed'; it.detail = 'SMTP: permanent error (554): 5.7.1 Message rejected'; }
          else { it.state = 'accepted'; it.detail = `250 2.0.0 Ok: queued as ${qid}`; }
        }
        job.log.push(`E-mail ${m + 1} (${mail.length} book${mail.length === 1 ? '' : 's'}): 250 2.0.0 Ok: queued as ${qid}`);
      });
    });
  }
  let i = 0;
  broadcast('job', job);
  const timer = setInterval(() => {
    if (job.state === 'cancelled') { clearInterval(timer); return; }
    ticks[i]?.();
    i++;
    job.progress = Math.min(1, i / Math.max(1, ticks.length));
    if (i >= ticks.length) {
      clearInterval(timer);
      const failed = items.filter((it) => it.state === 'failed').length;
      const ok = items.length - failed;
      const mails = new Set(items.filter((it) => it.state === 'accepted').map((it) => it.mail)).size;
      job.finishedAt = new Date().toISOString();
      job.progress = 1;
      if (ok === 0) { job.state = 'failed'; job.message = `all ${items.length} items failed`; }
      else {
        job.state = 'done';
        job.message = opts.email
          ? (failed ? `${ok} of ${items.length} sent in ${mails} e-mail${mails === 1 ? '' : 's'}, ${failed} failed` : `${items.length} book${items.length === 1 ? '' : 's'} sent in ${mails} e-mail${mails === 1 ? '' : 's'}`)
          : `${items.length} done`;
      }
      job.retryable = job.state === 'failed' || failed > 0;
      if (opts.email && opts.kindle && ok > 0) {
        job.hint = {
          code: 'kindle_approved_sender', from: store.settings.smtp.from,
          url: 'https://www.amazon.com/hz/mycd/myx#/home/settings/payment',
          text: `If it doesn't arrive in a few minutes, check that ${store.settings.smtp.from} is in Amazon's Approved Personal Document E-mail List`,
        };
      }
      if (!opts.email) job.downloadUrl = `/api/v1/jobs/${job.id}/download`;
    }
    broadcast('job', job);
  }, step);
}
