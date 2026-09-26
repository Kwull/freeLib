// Mock of the server's "find the next book" features: word forms / transliteration / typo
// tolerance in search, editions of one work, the start page and follows (docs/web/API.md).
// Simplified versions of freelib_catalog::{text, vocab, works, home}.
import { normalize } from './normalize';
import type { MockBook, MockLibrary } from './gen';

// ---------------------------------------------------------------- user state
/** `${lib}:${bookId}` → when it was sent / downloaded / read (the user's history). */
export const history = new Map<string, string>([
  // a reader who started two series and read Asimov (lib 1 ids are looked up by title below)
]);
export const follows = new Map<number, { authors: Set<number>; series: Set<number> }>();
export const dismissed = new Map<number, Set<number>>();
let seeded = false;

/** The default mock user has read a few books (so the start page has content). */
export function seedHistory(lib: MockLibrary) {
  if (seeded || lib.id !== 1) return;
  seeded = true;
  const byTitle = (t: string) => lib.books.find((b) => b.title === t && b.lang === 'ru');
  for (const [t, at] of [['Полдень, XXII век', '2026-09-01T10:00:00Z'], ['Попытка к бегству', '2026-09-08T10:00:00Z'], ['Я, робот', '2026-08-20T10:00:00Z']] as const) {
    const b = byTitle(t);
    if (b) history.set(`${lib.id}:${b.id}`, at);
  }
  const holmes = lib.books.find((b) => b.title === 'A Study in Scarlet');
  if (holmes) history.set(`${lib.id}:${holmes.id}`, '2026-07-01T10:00:00Z');
}

export function recordHistory(lib: number, ids: number[]) {
  const now = new Date().toISOString();
  for (const id of ids) history.set(`${lib}:${id}`, now);
}

// ---------------------------------------------------------------- text
const TR: Record<string, string> = {
  а: 'a', б: 'b', в: 'v', г: 'g', ґ: 'g', д: 'd', е: 'e', ё: 'e', є: 'ye', ж: 'zh', з: 'z', и: 'i', і: 'i', ї: 'yi', й: 'y',
  к: 'k', л: 'l', м: 'm', н: 'n', о: 'o', п: 'p', р: 'r', с: 's', т: 't', у: 'u', ф: 'f', х: 'kh', ц: 'ts', ч: 'ch',
  ш: 'sh', щ: 'shch', ъ: '', ь: '', ы: 'y', э: 'e', ю: 'yu', я: 'ya',
};
/** Latin key of a normalized word (the server's `word_key`, slightly simplified). */
export function latinKey(w: string): string {
  let s = [...w].map((c) => TR[c] ?? c).join('').replace(/[^a-z0-9]/g, '');
  s = s.replace(/tch/g, 'ch').replace(/kh/g, 'h').replace(/ks/g, 'x').replace(/tz|ts|tc/g, 'c')
    .replace(/j/g, 'y').replace(/iy|ii|yi/g, 'y');
  if (s.startsWith('ye')) s = 'e' + s.slice(2);
  s = s.replace(/([aeiou])y([aeiou])/g, '$1$2').replace(/([^aeiou])y([aeiou])/g, '$1i$2').replace(/([aeiou])y$/, '$1i');
  return s;
}
/** A crude word-form stem (the server uses Snowball Russian / English). */
export function stem(w: string): string {
  if (w.length < 4) return w;
  const s = w.replace(/(ами|ями|ого|ему|ому|ыми|ими|ах|ях|ов|ев|ей|ой|ий|ый|ая|яя|ое|ее|ам|ям|ом|ем|ую|юю|[аяоеиыуюйь])$/u, '');
  return s.length >= 3 ? s : w;
}
export const words = (s: string) => normalize(s).split(/[^\p{L}\p{N}]+/u).filter(Boolean);

type WordInfo = { w: string[]; s: string[]; k: string[] };
const infoCache = new Map<string, WordInfo>();
function info(text: string): WordInfo {
  let i = infoCache.get(text);
  if (!i) {
    const w = words(text);
    i = { w, s: w.map(stem), k: w.map(latinKey) };
    infoCache.set(text, i);
  }
  return i;
}
/** How a query token matches word `j`: 3 prefix, 1 word form or transliteration, 0 not. */
function tokenMatch(tok: string, ts: string, tk: string, i: WordInfo, j: number): number {
  if (i.w[j].startsWith(tok)) return 3;
  if (tok.length >= 3 && ts === i.s[j]) return 1;
  if (tok.length >= 3 && tk.length >= 3 && (tok.length >= 4 ? i.k[j].startsWith(tk) : i.k[j] === tk)) return 1;
  return 0;
}
/** Tier of `text` for the query tokens: 2 all prefixes, 1 some by form/translit, 0 no match. */
export function matchTier(tokens: string[], text: string): number {
  const i = info(text);
  let tier = 2;
  for (const t of tokens) {
    const ts = stem(t), tk = latinKey(t);
    let best = 0;
    for (let j = 0; j < i.w.length && best < 3; j++) best = Math.max(best, tokenMatch(t, ts, tk, i, j));
    if (best === 0) return 0;
    if (best === 1) tier = 1;
  }
  return tier;
}
export function queryTokens(q: string): string[] {
  const all = words(q);
  const long = all.filter((t) => t.length >= 2);
  return long.length ? long : all;
}
export function highlightWords(tokens: string[], texts: string[]): string[] {
  const out = new Set<string>();
  for (const s of texts) {
    const i = info(s);
    i.w.forEach((w, j) => { if (tokens.some((t) => tokenMatch(t, stem(t), latinKey(t), i, j) > 0)) out.add(w); });
  }
  return [...out];
}

function lev(a: string, b: string, max: number): number {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  let prev = Array.from({ length: b.length + 1 }, (_, i) => i);
  for (let i = 1; i <= a.length; i++) {
    const cur = [i];
    for (let j = 1; j <= b.length; j++) cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
    prev = cur;
  }
  return prev[b.length];
}
const vocabCache = new Map<number, Map<string, number>>();
function vocab(lib: MockLibrary): Map<string, number> {
  let v = vocabCache.get(lib.id);
  if (!v) {
    v = new Map();
    const add = (s: string, n = 1) => { for (const w of words(s)) if (w.length >= 3) v!.set(w, (v!.get(w) ?? 0) + n); };
    for (const b of lib.books) add(b.title);
    for (const a of lib.authors) add(a.name, a.bookCount);
    for (const s of lib.series) add(s.name, s.bookCount);
    vocabCache.set(lib.id, v);
  }
  return v;
}
/** The query with unknown words replaced by the closest vocabulary word, or null. */
export function correct(lib: MockLibrary, q: string): string | null {
  const v = vocab(lib);
  let changed = false;
  const out = words(q).map((t) => {
    if (t.length < 4) return t;
    const k = latinKey(t);
    for (const w of v.keys()) if (w.startsWith(t) || (k.length >= 3 && latinKey(w).startsWith(k))) return t;
    const max = t.length <= 7 ? 1 : 2;
    let best: [string, number, number] | null = null;
    for (const [w, n] of v) {
      const d = Math.min(lev(t, w, max), lev(k, latinKey(w), max));
      if (d <= max && (!best || d < best[1] || (d === best[1] && n > best[2]))) best = [w, d, n];
    }
    if (!best) return t;
    changed = true;
    return best[0];
  });
  return changed ? out.join(' ') : null;
}

// ---------------------------------------------------------------- editions
const EDITION_NOTE = /\s*[([]([^)\]]*(перевод|пер\.|иллюстр|редакц|translat|версия|издани)[^)\]]*)[)\]]\s*$/iu;
export function editionNote(b: MockBook): string | null {
  const m = b.title.match(EDITION_NOTE);
  const parts = [m?.[1], b.keywords && /перев|translat/i.test(b.keywords) ? b.keywords : undefined].filter(Boolean);
  return parts.length ? parts.join('; ') : null;
}
export function workKey(b: MockBook): string {
  const t = normalize(b.title.replace(EDITION_NOTE, ''));
  return `${b.lang}|${t}|${[...b.authorIds].sort((x, y) => x - y).join(',')}`;
}
/** Best copy first: live, FB2, then EPUB, larger (20 % steps, ≤ 30 MB), newer, lower id. */
export function editionCmp(a: MockBook, b: MockBook): number {
  const fmt = (x: MockBook) => (x.ext === 'fb2' ? 0 : x.ext === 'epub' ? 1 : 2);
  const bucket = (x: MockBook) => Math.floor(Math.log(Math.min(Math.max(x.size, 1), 30 << 20)) / Math.log(1.2));
  return Number(a.deleted) - Number(b.deleted) || fmt(a) - fmt(b) || bucket(b) - bucket(a)
    || (a.date < b.date ? 1 : a.date > b.date ? -1 : 0) || a.id - b.id;
}
export type Grouped = { best: MockBook; members: MockBook[] };
/** One entry per work at its first position; members best first. */
export function groupBooks(items: MockBook[]): Grouped[] {
  const pos = new Map<string, Grouped>();
  const out: Grouped[] = [];
  for (const b of items) {
    const k = workKey(b);
    const g = pos.get(k);
    if (g) g.members.push(b);
    else { const ng = { best: b, members: [b] }; pos.set(k, ng); out.push(ng); }
  }
  for (const g of out) {
    if (g.members.length < 2) continue;
    g.members.sort(editionCmp);
    g.best = g.members[0];
  }
  return out;
}
export function editionsOf(lib: MockLibrary, id: number): MockBook[] | null {
  const b = lib.bookById.get(id);
  if (!b) return null;
  const k = workKey(b);
  return lib.books.filter((x) => (x.id === id || !x.deleted) && workKey(x) === k).sort(editionCmp);
}

// ---------------------------------------------------------------- start page
export function followsOf(lib: number) {
  let f = follows.get(lib);
  if (!f) { f = { authors: new Set(), series: new Set() }; follows.set(lib, f); }
  return f;
}

/** `n` days before `from` (the mock library's newest book date: the data does not move). */
function daysAgo(n: number, from: string): string {
  const d = new Date(`${from}T00:00:00Z`);
  d.setUTCDate(d.getUTCDate() - n);
  return d.toISOString().slice(0, 10);
}

export function homeOf(lib: MockLibrary, opts: { days: number | null; ratings: Map<string, number>; fresh: boolean }) {
  seedHistory(lib);
  const done = new Map<number, string>();
  if (!opts.fresh) {
    for (const [k, at] of history) { const [l, id] = k.split(':').map(Number); if (l === lib.id) done.set(id, at); }
    for (const [k, r] of opts.ratings) { const [l, id] = k.split(':').map(Number); if (l === lib.id && r > 0 && !done.has(id)) done.set(id, ''); }
  }
  const f = opts.fresh ? { authors: new Set<number>(), series: new Set<number>() } : followsOf(lib.id);
  const gone = opts.fresh ? new Set<number>() : (dismissed.get(lib.id) ?? new Set());

  // continue series
  const bySeries = new Map<number, string>();
  for (const [id, at] of done) {
    const b = lib.bookById.get(id);
    if (b?.seriesId && (bySeries.get(b.seriesId) ?? '') <= at) bySeries.set(b.seriesId, at);
  }
  const continueSeries = [];
  for (const [sid, lastAt] of [...bySeries].sort((a, b) => (a[1] < b[1] ? 1 : -1))) {
    if (gone.has(sid)) continue;
    const s = lib.series[sid - 1];
    let all = (lib.booksBySeries.get(sid) ?? []).map((id) => lib.bookById.get(id)!);
    // a publisher series (many first authors): only the books by the authors read in it
    if (new Set(all.map((b) => b.authorIds[0])).size > 3) {
      const mine = new Set(all.filter((b) => done.has(b.id)).flatMap((b) => b.authorIds));
      all = all.filter((b) => done.has(b.id) || b.authorIds.some((a) => mine.has(a)));
    }
    const books = all.filter((b) => !b.deleted || done.has(b.id))
      .sort((a, b) => (a.serno ?? 1e9) - (b.serno ?? 1e9) || (a.sortKey < b.sortKey ? -1 : 1));
    const groups = groupBooks(books);
    const isDone = groups.map((g) => g.members.some((m) => done.has(m.id)));
    const last = isDone.lastIndexOf(true);
    if (last < 0) continue;
    const next = groups.slice(last + 1).filter((_, i) => !isDone[last + 1 + i]).slice(0, 2);
    if (!next.length) continue;
    const authors = [...new Set(books.flatMap((b) => b.authorIds.slice(0, 1)))].slice(0, 2).map((a) => lib.authors[a - 1].name).join(', ');
    continueSeries.push({ series: { id: sid, name: s.name, count: s.bookCount, authors }, works: groups.length, done: isDone.filter(Boolean).length, lastAt, next });
    if (continueSeries.length >= 24) break;
  }

  // new from authors
  const readAuthors = new Set<number>();
  for (const id of done.keys()) {
    const b = lib.bookById.get(id);
    if (b && b.authorIds.length < 4) for (const a of b.authorIds) readAuthors.add(a);
  }
  const newest = lib.books.reduce((m, b) => (b.date > m ? b.date : m), '');
  const since = daysAgo(opts.days ?? 30, newest);
  const doneWorks = new Set([...done.keys()].map((id) => workKey(lib.bookById.get(id)!)));
  const fresh = lib.books.filter((b) => !b.deleted && b.date >= since && !doneWorks.has(workKey(b))
    && (b.authorIds.some((a) => readAuthors.has(a) || f.authors.has(a)) || (b.seriesId !== null && f.series.has(b.seriesId))))
    .sort((a, b) => (a.date < b.date ? 1 : a.date > b.date ? -1 : a.id - b.id));
  const groups = groupBooks(fresh);
  const newBooks = groups.slice(0, 60).map((g) => {
    const b = g.best;
    const reason = b.seriesId !== null && f.series.has(b.seriesId)
      ? { kind: 'series', id: b.seriesId, name: lib.series[b.seriesId - 1].name, followed: true }
      : (() => {
          const fa = b.authorIds.find((a) => f.authors.has(a));
          if (fa) return { kind: 'author', id: fa, name: lib.authors[fa - 1].name, followed: true };
          const ra = b.authorIds.find((a) => readAuthors.has(a)) ?? 0;
          return { kind: 'author', id: ra, name: ra ? lib.authors[ra - 1].name : '', followed: false };
        })();
    return { g, reason };
  });
  const empty = done.size === 0 && f.authors.size === 0 && f.series.size === 0;
  let picks: Grouped[] = [];
  if (empty || (!continueSeries.length && !newBooks.length)) {
    const recent = lib.books.filter((b) => !b.deleted && b.date >= daysAgo(30, newest));
    const star = (b: MockBook) => (b.id * 2654435761 >>> 0) % 6;
    picks = groupBooks(recent.sort((a, b) => star(b) - star(a) || (a.date < b.date ? 1 : -1))).slice(0, 12);
  }
  return { empty, continueSeries, newBooks, newTotal: groups.length, since, picks, following: { authors: f.authors.size, series: f.series.size } };
}
