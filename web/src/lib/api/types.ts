// Types mirrored from docs/web/API.md — keep in sync with the server contract.

export type LibraryStatus = {
  state: 'idle' | 'importing' | 'error'; progress?: number; message?: string;
  /** An import the server started by itself: `upgrade` = catalog rebuilt after an update. */
  reason?: 'upgrade' | 'autoimport';
};

export type Library = {
  id: number; name: string; path: string; inpx: string | null;
  firstAuthorOnly: boolean; skipDeleted: boolean; isDefault: boolean;
  bookCount: number; authorCount: number; seriesCount: number;
  importedAt: string | null;
  catalogVersion: number;
  newSinceLastVisit: number;
  status: LibraryStatus;
  opdsUrl: string;
  /** Open Library lookups of this library's books (absent on older servers). */
  externalRatings?: ExtProgress;
};

/** Progress of the Open Library enrichment. */
export type ExtProgress = { lookedUp: number; found: number; rated: number };

export type AuthorRef = { id: number; name: string };
export type SeriesRef = { id: number; name: string };

export type Book = {
  id: number; key: string;
  title: string;
  authors: AuthorRef[];
  series: SeriesRef | null; serno: number | null;
  genres: number[];
  lang: string; ext: string; size: number;
  date: string;
  deleted: boolean;
  /** my rating 0..5 (0 = not rated) */
  rating: number;
  shelves: number[];
  /** library (INPX) rating 0..5, 0 = none */
  libRating: number;
  /** Open Library average and vote count, when known and rated */
  extRating: { avg: number; votes: number } | null;
  /** age suitability estimate (heuristic): 0, 6, 12, 16, 18; null = unknown */
  kidsAge: number | null;
};

/** The cached Open Library lookup of a book. */
export type ExtRatingInfo = {
  source: 'openlibrary';
  status: 'found' | 'not_found' | 'error';
  average: number | null;
  count: number;
  workKey: string | null;
  url: string | null;
  fetchedAt: string;
};

/** Rating filters / sort of book lists and search (server side). */
export type RatingParams = {
  sort?: 'my' | 'lib' | 'ext';
  minMy?: number; minLib?: number; minExt?: number; minExtVotes?: number;
  unratedByMe?: boolean; kidsMaxAge?: number;
};

export type ApiToken = {
  id: number; name: string; prefix: string; scopes: TokenScope[];
  createdAt: string; lastUsedAt: string | null; expiresAt: string | null;
};
export type TokenScope = 'read' | 'write' | 'send';
export type TokensResponse = { tokens: ApiToken[]; scopes: TokenScope[]; mcp: { enabled: boolean; url: string } };
export type AuditRow = {
  id: number; tokenId: number | null; tokenName: string | null; tool: string; ok: boolean; detail: string; at: string;
};

export type BookDetail = Book & {
  extRatingInfo?: ExtRatingInfo | null;
  annotation: string | null;
  hasCover: boolean;
  file: string;
  keywords: string;
  formats: string[];
};

export type Genre = { id: number; name: string; parent: number; count: number };
export type Shelf = { id: number; name: string; color: string; count: number };

export type DeviceKind = 'email' | 'download' | 'folder';
export type DeviceFormat = 'original' | 'epub' | 'kepub' | 'azw3' | 'mobi' | 'pdf';

export type Device = {
  id: number; name: string;
  kind: DeviceKind;
  format: DeviceFormat;
  target: string | null;
  fileName: string;
  shared: boolean;
  options: ConvertOptions;
};

export type ConvertOptions = {
  hyphenate: 'none' | 'soft' | 'full';
  footnotes: 'end' | 'inline' | 'popup';
  dropCaps: boolean;
  breakAfterChapter: boolean;
  tocPlacement: 'start' | 'end' | 'none';
  createCover: 'never' | 'missing' | 'always';
  coverLabel: string | null;
  joinSeries: boolean;
  transliterate: boolean;
  annotation: boolean;
  fontFamily: string | null;
  userCss: string | null;
};

export type JobKind = 'import' | 'send' | 'export' | 'download';
export type JobState = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';

export type Job = {
  id: string; kind: JobKind;
  title: string;
  state: JobState;
  progress: number;
  message: string;
  log: string[];
  downloadUrl: string | null;
  createdAt: string; finishedAt: string | null;
};

export type User = { id: number; username: string; role: 'admin' | 'reader' };
export type AuthMethods = {
  /** Password sign-in offered in the web app (`FREELIB_OIDC_DISABLE_PASSWORD` turns it off). */
  password: boolean;
  /** Single sign-on (OpenID Connect), when configured. */
  oidc: { enabled: boolean; label: string } | null;
};
export type Session = { user: User | null; openMode: boolean; auth?: AuthMethods };

/** A user in the admin's list: how they sign in. */
export type UserRow = User & {
  hasPassword: boolean;
  sso: { issuer: string; email: string | null; createdAt: string; lastLogin: string | null } | null;
};

/** GET /me/account */
export type Account = {
  user: User;
  hasPassword: boolean;
  /** Whether password sign-in is allowed for this user in the web app. */
  passwordLogin: boolean;
  sso: { label: string; linked: boolean; email: string | null; lastLogin: string | null } | null;
};

export type NameListResponse = {
  version: number;
  columns: ['id', 'name', 'count'];
  rows: [number, string, number][];
  letters: [string, number, number][];
};

export type Coauthor = { id: number; name: string; books: number; direct: number };

export type AuthorSummary = {
  id: number; name: string;
  count: number;
  /** live books with ≥ 4 authors (anthologies / collections) */
  anthologies: number;
  series: { id: number; name: string; count: number }[];
  withoutSeries: number;
  langs: [string, number][];
  genres: [number, number][];
  firstDate: string; lastDate: string;
  /** top co-authors (≥ 1 direct shared book or ≥ 2 shared books), at most 10 */
  coauthors: Coauthor[];
  coauthorCount: number;
};

export type CoauthorsResponse = { columns: ['id', 'name', 'books', 'direct']; rows: [number, string, number, number][] };

export type BooksResponse = { books: Book[]; nextCursor: string | null; total: number };

export type SearchResponse = {
  tookMs: number;
  authors: { id: number; name: string; count: number }[];
  series: { id: number; name: string; count: number; authors: string }[];
  books: Book[];
  total: number;
  facets: { genre: [number, number][]; lang: [string, number][]; ext: [string, number][] };
};

export type SmtpSettings = {
  host: string; port: number; security: 'none' | 'starttls' | 'tls';
  username: string; from: string; passwordSet: boolean; pauseSeconds: number;
  /** Recipient patterns (`*` = any characters) that sending by e-mail is limited to. */
  allowedRecipients: string[];
  dailyLimitPerUser: number;
  /** Mail subject template: `%b` = book title, `%a` = author(s). */
  subject: string;
  /** Write-only: send to change the SMTP password, `''` clears it, omit to keep it. */
  password?: string;
};
export type Settings = {
  smtp: SmtpSettings;
  opds: { enabled: boolean; requireAuth: boolean };
  calibre: { available: boolean; version: string | null };
  /** Open Library ratings: the admin switch and the enrichment progress (read-only parts) */
  externalRatings?: {
    enabled: boolean; source?: string; contactSet?: boolean;
    progress?: ExtProgress & { total: number };
    queued?: number; requests?: number; pausedFor?: number; lastError?: string | null;
  };
  /** the MCP endpoint (/mcp) */
  mcp?: { enabled: boolean; url?: string | null };
};

export type ApiErrorCode =
  | 'unauthorized' | 'forbidden' | 'not_found' | 'bad_request' | 'conflict' | 'unsupported_format'
  | 'rate_limited' | 'internal';

export class ApiError extends Error {
  code: ApiErrorCode;
  status: number;
  constructor(status: number, code: ApiErrorCode, message: string) {
    super(message);
    this.status = status;
    this.code = code;
    this.name = 'ApiError';
  }
}
