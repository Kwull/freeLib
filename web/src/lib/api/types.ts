// Types mirrored from docs/web/API.md — keep in sync with the server contract.

export type LibraryStatus = { state: 'idle' | 'importing' | 'error'; progress?: number; message?: string };

export type Library = {
  id: number; name: string; path: string; inpx: string | null;
  firstAuthorOnly: boolean; skipDeleted: boolean; isDefault: boolean;
  bookCount: number; authorCount: number; seriesCount: number;
  importedAt: string | null;
  catalogVersion: number;
  newSinceLastVisit: number;
  status: LibraryStatus;
  opdsUrl: string;
};

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
  rating: number;
  shelves: number[];
};

export type BookDetail = Book & {
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
export type Session = { user: User | null; openMode: boolean };

export type NameListResponse = {
  version: number;
  columns: ['id', 'name', 'count'];
  rows: [number, string, number][];
  letters: [string, number, number][];
};

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
};
export type Settings = {
  smtp: SmtpSettings;
  opds: { enabled: boolean; requireAuth: boolean };
  calibre: { available: boolean; version: string | null };
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
