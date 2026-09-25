import { ApiError } from './types';
import type {
  Library, Book, BookDetail, Genre, Shelf, Device, Job, Session, NameListResponse,
  BooksResponse, SearchResponse, Settings, User, ConvertOptions, AuthorSummary, CoauthorsResponse,
  Account, UserRow,
} from './types';

const BASE = '/api/v1';

async function failure(res: Response): Promise<ApiError> {
  const isJson = res.headers.get('content-type')?.includes('application/json');
  let code = 'internal', message = res.statusText || `HTTP ${res.status}`;
  if (isJson) {
    try {
      const body = await res.json();
      code = body.error ?? code;
      message = body.message ?? message;
    } catch { /* ignore */ }
  }
  return new ApiError(res.status, code as any, message);
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(BASE + path, {
    credentials: 'include',
    headers: init?.body ? { 'Content-Type': 'application/json' } : undefined,
    ...init,
  });
  if (res.status === 204) return undefined as T;
  if (!res.ok) throw await failure(res);
  const isJson = res.headers.get('content-type')?.includes('application/json');
  if (isJson) return (await res.json()) as T;
  return undefined as T;
}

export type StreamProgress = { bytes: number; rows: number };

/**
 * GET of a JSON body read as a stream, reporting bytes received and the rows of a
 * `[[…],[…],…]` array seen so far (a count of `],[`), for big lists on slow links.
 */
async function requestStreamed<T>(path: string, onProgress: (p: StreamProgress) => void): Promise<T> {
  const res = await fetch(BASE + path, { credentials: 'include' });
  if (!res.ok) throw await failure(res);
  if (!res.body) return (await res.json()) as T;
  const reader = res.body.getReader();
  const chunks: Uint8Array[] = [];
  let bytes = 0, rows = 0, prev1 = 0, prev2 = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    bytes += value.length;
    for (let i = 0; i < value.length; i++) {
      const b = value[i];
      if (b === 91 /* [ */ && prev1 === 44 /* , */ && prev2 === 93 /* ] */) rows++;
      prev2 = prev1; prev1 = b;
    }
    onProgress({ bytes, rows });
  }
  const all = new Uint8Array(bytes);
  let off = 0;
  for (const c of chunks) { all.set(c, off); off += c.length; }
  return JSON.parse(new TextDecoder().decode(all)) as T;
}

/** Human-readable text of a failed request (the server's message for API errors). */
export function errorText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

function qs(params: Record<string, string | number | boolean | undefined | null>): string {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v !== undefined && v !== null && v !== '') p.set(k, String(v));
  }
  const s = p.toString();
  return s ? `?${s}` : '';
}

export const api = {
  // Session
  session: () => request<Session>('/session'),
  login: (username: string, password: string) =>
    request<{ user: User }>('/login', { method: 'POST', body: JSON.stringify({ username, password }) }),
  logout: () => request<void>('/logout', { method: 'POST' }),
  /** Single sign-on: where the browser goes to sign in (a full page navigation). */
  oidcLoginUrl: (returnTo: string) => `${BASE}/auth/oidc/login${qs({ return: returnTo })}`,
  oidcLink: () => request<{ url: string }>('/auth/oidc/link', { method: 'POST' }),
  oidcUnlink: () => request<void>('/me/oidc', { method: 'DELETE' }),
  account: () => request<Account>('/me/account'),
  setPassword: (password: string, current?: string) =>
    request<void>('/me/password', { method: 'PUT', body: JSON.stringify({ password, current }) }),

  // Libraries
  libraries: () => request<Library[]>('/libraries'),
  createLibrary: (body: Partial<Library> & { name: string; path: string }) =>
    request<Library>('/libraries', { method: 'POST', body: JSON.stringify(body) }),
  updateLibrary: (id: number, body: Partial<Library>) =>
    request<Library>(`/libraries/${id}`, { method: 'PATCH', body: JSON.stringify(body) }),
  deleteLibrary: (id: number) => request<void>(`/libraries/${id}`, { method: 'DELETE' }),
  importLibrary: (id: number, mode: 'full' | 'new') =>
    request<Job>(`/libraries/${id}/import`, { method: 'POST', body: JSON.stringify({ mode }) }),
  fs: (path?: string) => request<{ path: string; parent: string | null; entries: { name: string; dir: boolean; size: number }[] }>(
    `/fs${qs({ path })}`,
  ),

  // Browsing
  authors: (lib: number, v?: number) => request<NameListResponse>(`/libraries/${lib}/authors${qs({ v })}`),
  series: (lib: number, v?: number) => request<NameListResponse>(`/libraries/${lib}/series${qs({ v })}`),
  /** Authors or series list, streamed with progress. */
  nameList: (lib: number, kind: 'authors' | 'series', v: number | undefined, onProgress: (p: StreamProgress) => void) =>
    requestStreamed<NameListResponse>(`/libraries/${lib}/${kind}${qs({ v })}`, onProgress),
  genres: (lib: number, lang?: string) => request<Genre[]>(`/libraries/${lib}/genres${qs({ lang })}`),
  books: (lib: number, params: {
    author?: number; series?: number; genre?: number; shelf?: number; since?: string;
    lang?: string; ext?: string; deleted?: boolean; q?: string; cursor?: string; limit?: number;
  }, init?: RequestInit) => request<BooksResponse>(`/libraries/${lib}/books${qs({ ...params, deleted: params.deleted ? 1 : undefined })}`, init),
  authorSummary: (lib: number, id: number) => request<AuthorSummary>(`/libraries/${lib}/authors/${id}/summary`),
  coauthors: (lib: number, id: number) => request<CoauthorsResponse>(`/libraries/${lib}/authors/${id}/coauthors`),
  book: (lib: number, id: number) => request<BookDetail>(`/libraries/${lib}/books/${id}`),
  coverUrl: (lib: number, id: number, size: 'thumb' | 'full' = 'thumb') =>
    `${BASE}/libraries/${lib}/books/${id}/cover?size=${size}`,
  fileUrl: (lib: number, id: number, format = 'original', opts?: { device?: number; inline?: boolean }) =>
    `${BASE}/libraries/${lib}/books/${id}/file${qs({ format, device: opts?.device, inline: opts?.inline ? 1 : undefined })}`,
  search: (lib: number, params: {
    q: string; kind?: 'all' | 'books' | 'authors' | 'series'; genre?: string; lang?: string;
    ext?: string; from?: string; to?: string; limit?: number;
  }) => request<SearchResponse>(`/libraries/${lib}/search${qs(params)}`),
  languages: (lib: number) => request<[string, number][]>(`/languages${qs({ lib })}`),
  setRating: (lib: number, id: number, rating: number) =>
    request<void>(`/libraries/${lib}/books/${id}/rating`, { method: 'PUT', body: JSON.stringify({ rating }) }),

  // Shelves
  shelves: () => request<Shelf[]>('/shelves'),
  createShelf: (name: string, color: string) =>
    request<Shelf>('/shelves', { method: 'POST', body: JSON.stringify({ name, color }) }),
  updateShelf: (id: number, body: { name?: string; color?: string }) =>
    request<Shelf>(`/shelves/${id}`, { method: 'PATCH', body: JSON.stringify(body) }),
  deleteShelf: (id: number) => request<void>(`/shelves/${id}`, { method: 'DELETE' }),
  shelfBooks: (id: number, library: number, books: number[], add: boolean) =>
    request<Shelf>(`/shelves/${id}/books`, { method: 'POST', body: JSON.stringify({ library, books, add }) }),

  // Devices / sending
  devices: () => request<Device[]>('/devices'),
  createDevice: (d: Omit<Device, 'id'>) => request<Device>('/devices', { method: 'POST', body: JSON.stringify(d) }),
  updateDevice: (id: number, d: Device) => request<Device>(`/devices/${id}`, { method: 'PUT', body: JSON.stringify(d) }),
  deleteDevice: (id: number) => request<void>(`/devices/${id}`, { method: 'DELETE' }),
  send: (body: {
    library: number; books: number[]; device: number; target?: string; fileName?: string;
    options?: Partial<ConvertOptions>;
  }) => request<Job>('/send', { method: 'POST', body: JSON.stringify(body) }),
  fonts: () => request<string[]>('/fonts'),

  // Jobs
  jobs: () => request<Job[]>('/jobs'),
  cancelJob: (id: string) => request<Job>(`/jobs/${id}/cancel`, { method: 'POST' }),
  clearFinishedJobs: () => request<void>('/jobs?finished=1', { method: 'DELETE' }),
  jobDownloadUrl: (id: string) => `${BASE}/jobs/${id}/download`,

  // Settings / users
  settings: () => request<Settings>('/settings'),
  updateSettings: (s: Settings) => request<Settings>('/settings', { method: 'PUT', body: JSON.stringify(s) }),
  testSmtp: (to: string) => request<void>('/settings/smtp/test', { method: 'POST', body: JSON.stringify({ to }) }),
  users: () => request<UserRow[]>('/users'),
  createUser: (u: { username: string; password: string; role: string }) =>
    request<User>('/users', { method: 'POST', body: JSON.stringify(u) }),
  updateUser: (id: number, u: { password?: string; role?: string }) =>
    request<User>(`/users/${id}`, { method: 'PATCH', body: JSON.stringify(u) }),
  deleteUser: (id: number) => request<void>(`/users/${id}`, { method: 'DELETE' }),

  prefs: () => request<Record<string, unknown>>('/me/prefs'),
  setPrefs: (p: Record<string, unknown>) => request<Record<string, unknown>>('/me/prefs', { method: 'PUT', body: JSON.stringify(p) }),
};

export { ApiError };
