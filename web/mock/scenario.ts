// Simulated server states for the loading / importing / error / sign-in screens.
//
// A scenario is picked per browser by the `freelib_mock` cookie (URL-encoded query string:
// `s=rebuild,slow&dur=8000`), or for the whole dev server by `MOCK_SCENARIO` (same syntax).
// Open `/__mock?s=rebuild,slow` in the dev server to set the cookie and go to the app;
// `/__mock` alone clears it. Tokens of `s`:
//
//   rebuild      library 1 is rebuilt after an update (status.reason = "upgrade") for `dur` ms
//   importing    library 1 is imported for the first time for `dur` ms
//   importfail   library 1's import failed (status error, no catalog)
//   nolibs       no libraries at all (first run)
//   libserror    GET /libraries answers 500
//   sessionerror GET /session answers 503
//   sessionhang  GET /session answers after 60 s
//   slow         authors/series lists trickle in over `slow` ms (default 4000)
//   nosse        GET /events fails (the app falls back to polling)
//   reader       signed in as a reader (not open mode)
//   loggedout    not signed in (login page)
//   sso          single sign-on configured (with loggedout: the login page shows the button)
//   nopassword   password sign-in disabled (FREELIB_OIDC_DISABLE_PASSWORD)
import type { IncomingMessage, ServerResponse } from 'node:http';
import type { Library } from '../src/lib/api/types';

export type Scenario = { flags: Set<string>; dur: number; slow: number; key: string };

const started = new Map<string, number>();

export function scenarioOf(req: IncomingMessage): Scenario | null {
  let raw = process.env.MOCK_SCENARIO ?? '';
  const cookie = (req.headers.cookie ?? '').split(';').map((c) => c.trim()).find((c) => c.startsWith('freelib_mock='));
  if (cookie) raw = decodeURIComponent(cookie.slice('freelib_mock='.length));
  if (!raw) return null;
  const q = new URLSearchParams(raw.includes('=') ? raw : `s=${raw}`);
  const flags = new Set((q.get('s') ?? '').split(',').map((s) => s.trim()).filter(Boolean));
  if (!flags.size) return null;
  const key = raw;
  if (!started.has(key)) started.set(key, Date.now());
  return { flags, dur: Number(q.get('dur') ?? 8000) || 8000, slow: Number(q.get('slow') ?? 4000) || 4000, key };
}

/** 0..1 of the simulated import, 1 when finished. */
function importProgress(sc: Scenario): number {
  return Math.min(1, (Date.now() - (started.get(sc.key) ?? Date.now())) / sc.dur);
}

const STEPS = ['Reading INPX', 'Parsing records', 'Writing books', 'Building the search index', 'Sorting names', 'Finishing'];

/** Library 1 as the scenario shows it (`null`: unchanged). */
export function scenarioLibrary(sc: Scenario, lib: Library): Library {
  if (lib.id !== 1) return lib;
  if (sc.flags.has('importfail')) {
    return {
      ...lib, importedAt: null, catalogVersion: 0, bookCount: 0, authorCount: 0, seriesCount: 0,
      status: { state: 'error', message: 'import failed: cannot open /books/flibusta/flibusta.inpx: No such file or directory (os error 2)' },
    };
  }
  if (sc.flags.has('rebuild') || sc.flags.has('importing')) {
    const p = importProgress(sc);
    if (p >= 1) return { ...lib, catalogVersion: lib.catalogVersion + 1 };
    return {
      ...lib, importedAt: null, catalogVersion: 0, bookCount: 0, authorCount: 0, seriesCount: 0,
      status: {
        state: 'importing', progress: p, message: STEPS[Math.min(STEPS.length - 1, Math.floor(p * STEPS.length))],
        ...(sc.flags.has('rebuild') ? { reason: 'upgrade' as const } : {}),
      },
    };
  }
  return lib;
}

export function libraryBrowsable(sc: Scenario | null, libId: number): boolean {
  if (!sc || libId !== 1) return true;
  if (sc.flags.has('importfail')) return false;
  if (sc.flags.has('rebuild') || sc.flags.has('importing')) return importProgress(sc) >= 1;
  return true;
}

function json(res: ServerResponse, status: number, body: unknown) {
  res.statusCode = status;
  res.setHeader('Content-Type', 'application/json; charset=utf-8');
  res.end(JSON.stringify(body));
}

/** Answers `/__mock` (sets or clears the cookie, then opens the app). */
export function handleMockControl(url: URL, res: ServerResponse): boolean {
  if (url.pathname !== '/__mock') return false;
  const s = url.searchParams.toString();
  res.statusCode = 303;
  res.setHeader('Set-Cookie', s
    ? `freelib_mock=${encodeURIComponent(s)}; Path=/; SameSite=Lax`
    : 'freelib_mock=; Path=/; Max-Age=0');
  res.setHeader('Location', url.searchParams.get('to') ?? '/');
  res.end();
  return true;
}

/**
 * Scenario overrides that answer a request by themselves; `true` when handled. `libraries` is
 * the store's list (for `/libraries` and SSE).
 */
export async function handleScenario(
  sc: Scenario, req: IncomingMessage, res: ServerResponse, path: string, libraries: () => Library[],
  sessionUser: { id: number; username: string; role: 'admin' | 'reader' },
): Promise<boolean> {
  const f = sc.flags;
  if (path === '/api/v1/session') {
    if (f.has('sessionerror')) { json(res, 503, { error: 'internal', message: 'database is locked' }); return true; }
    if (f.has('sessionhang')) await new Promise((r) => setTimeout(r, 60_000));
    const loggedOut = f.has('loggedout');
    const sso = f.has('sso') ? { enabled: true, label: 'Sign in with Pocket ID' } : null;
    const auth = { password: !f.has('nopassword'), oidc: sso };
    if (loggedOut) { json(res, 200, { user: null, openMode: false, auth }); return true; }
    if (f.has('reader')) { json(res, 200, { user: { id: 2, username: 'reader', role: 'reader' }, openMode: false, auth }); return true; }
    if (sso) { json(res, 200, { user: sessionUser, openMode: false, auth }); return true; }
    return false;
  }
  if (path === '/api/v1/me/account' && (f.has('sso') || f.has('reader'))) {
    const user = f.has('reader') ? { id: 2, username: 'reader', role: 'reader' } : sessionUser;
    json(res, 200, {
      user, hasPassword: !f.has('ssouser'), passwordLogin: !f.has('nopassword'),
      sso: f.has('sso') ? { label: 'Sign in with Pocket ID', linked: f.has('ssouser'), email: f.has('ssouser') ? 'reader@example.org' : null, lastLogin: null } : null,
    });
    return true;
  }
  if (path === '/api/v1/auth/oidc/login') {
    // no provider in the mock: come back as if the provider had refused
    res.statusCode = 303;
    res.setHeader('Location', '/login?ssoError=provider');
    res.end();
    return true;
  }
  if (path === '/api/v1/libraries' && req.method === 'GET') {
    if (f.has('libserror')) { json(res, 500, { error: 'internal', message: 'app.db: disk I/O error' }); return true; }
    if (f.has('nolibs')) { json(res, 200, []); return true; }
    json(res, 200, libraries().map((l) => scenarioLibrary(sc, l)));
    return true;
  }
  const m = path.match(/^\/api\/v1\/libraries\/(\d+)\/(authors|series|genres|books|search)/);
  if (m && !libraryBrowsable(sc, Number(m[1]))) {
    json(res, 404, { error: 'not_found', message: 'library is not imported yet' });
    return true;
  }
  if (path === '/api/v1/events') {
    if (f.has('nosse')) { json(res, 503, { error: 'internal', message: 'no events' }); return true; }
    if (f.has('rebuild') || f.has('importing')) {
      res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache', Connection: 'keep-alive' });
      res.write(': hello\n\n');
      const tick = setInterval(() => {
        const lib = libraries().find((l) => l.id === 1);
        if (!lib) return;
        const cur = scenarioLibrary(sc, lib);
        res.write(`event: library\ndata: ${JSON.stringify(cur)}\n\n`);
        if (cur.status.state !== 'importing') clearInterval(tick);
      }, 400);
      req.on('close', () => clearInterval(tick));
      return true;
    }
  }
  return false;
}

/** Sends `body` in pieces over `ms` (the `slow` flag): a big list on a slow link. */
export async function sendSlowly(res: ServerResponse, body: unknown, ms: number) {
  const text = Buffer.from(JSON.stringify(body));
  res.statusCode = 200;
  res.setHeader('Content-Type', 'application/json; charset=utf-8');
  res.setHeader('Content-Length', String(text.length));
  const parts = 20;
  const size = Math.ceil(text.length / parts);
  for (let i = 0; i < parts; i++) {
    res.write(text.subarray(i * size, (i + 1) * size));
    await new Promise((r) => setTimeout(r, ms / parts));
  }
  res.end();
}
