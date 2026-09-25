import { mulberry32, pick, int } from './rng';
import { normalize, letterOf } from './normalize';
import {
  SURNAMES, FIRST_NAMES_M, FIRST_NAMES_F, PATRONYMIC_M, PATRONYMIC_F,
  SERIES_WORDS_A, SERIES_WORDS_B, TITLE_WORDS_A, TITLE_WORDS_B, GENRES, LANGS, EXTS,
} from './names';

export type MockAuthor = { id: number; name: string; sortKey: string; bookCount: number };
export type MockSeries = { id: number; name: string; sortKey: string; bookCount: number };
/** `en`/`ru`/`uk` names carried through so the server can localize on request. */
export type MockGenre = { id: number; en: string; ru: string; uk: string; parent: number; count: number };
export type MockBook = {
  id: number; key: string; title: string; sortKey: string;
  authorIds: number[]; seriesId: number | null; serno: number | null;
  genreIds: number[]; lang: string; ext: string; size: number; date: string; deleted: boolean;
  annotation?: string;
};

export type MockLibrary = {
  id: number;
  authors: MockAuthor[];
  series: MockSeries[];
  genres: MockGenre[];
  books: MockBook[];
  booksByAuthor: Map<number, number[]>;
  booksBySeries: Map<number, number[]>;
  bookById: Map<number, MockBook>;
  authorLetters: [string, number, number][];
  seriesLetters: [string, number, number][];
};

const AUTHOR_COUNT = 50_000;
const SERIES_COUNT = 4_000;
const BOOK_COUNT = 3_000;

function buildLetters(sortedKeys: string[]): [string, number, number][] {
  const out: [string, number, number][] = [];
  let cur = '', count = 0, first = 0;
  sortedKeys.forEach((k, i) => {
    const l = letterOf(k);
    if (l !== cur) {
      if (count > 0) out.push([cur, count, first]);
      cur = l; count = 0; first = i;
    }
    count++;
  });
  if (count > 0) out.push([cur, count, first]);
  return out;
}

function randDate(rand: () => number): string {
  const start = new Date('2022-01-01').getTime();
  const end = new Date('2026-09-24').getTime();
  const d = new Date(start + rand() * (end - start));
  return d.toISOString().slice(0, 10);
}

export function generateLibrary(id: number, seed: number): MockLibrary {
  const rand = mulberry32(seed);

  // --- authors -------------------------------------------------------
  const authorsRaw: { name: string; sortKey: string }[] = [];
  // A guaranteed, prototype-like cluster of authors so demos/tests are stable.
  const seedAuthors = [
    'Doyle Arthur Conan', 'Doyley Arthur', 'Doylan Frederick', 'Doyler Henry',
    'Doyleston Margaret', 'Doyne Robert', 'Doynton Grace', 'Doyce Samuel',
  ];
  for (const name of seedAuthors) authorsRaw.push({ name, sortKey: normalize(name) });

  for (let i = seedAuthors.length; i < AUTHOR_COUNT; i++) {
    const isF = rand() < 0.35;
    const last = pick(rand, SURNAMES) + (isF ? 'а' : '');
    const first = isF ? pick(rand, FIRST_NAMES_F) : pick(rand, FIRST_NAMES_M);
    const middle = isF ? pick(rand, PATRONYMIC_F) : pick(rand, PATRONYMIC_M);
    const name = `${last} ${first} ${middle}`;
    authorsRaw.push({ name, sortKey: normalize(name) });
  }
  authorsRaw.sort((a, b) => (a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0));
  const authors: MockAuthor[] = authorsRaw.map((a, i) => ({ id: i + 1, name: a.name, sortKey: a.sortKey, bookCount: 0 }));
  const authorLetters = buildLetters(authors.map((a) => a.sortKey));

  // --- series ----------------------------------------------------------
  const seriesRaw: { name: string; sortKey: string }[] = [];
  const seedSeries = ['Sherlock Holmes', 'Professor Challenger'];
  for (const name of seedSeries) seriesRaw.push({ name, sortKey: normalize(name) });
  for (let i = seedSeries.length; i < SERIES_COUNT; i++) {
    const name = `${pick(rand, SERIES_WORDS_A)} ${pick(rand, SERIES_WORDS_B)}`;
    seriesRaw.push({ name, sortKey: normalize(name) });
  }
  seriesRaw.sort((a, b) => (a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0));
  const series: MockSeries[] = seriesRaw.map((s, i) => ({ id: i + 1, name: s.name, sortKey: s.sortKey, bookCount: 0 }));
  const seriesLetters = buildLetters(series.map((s) => s.sortKey));

  const genres: MockGenre[] = GENRES.map((g, i) => ({
    id: i + 1, en: g.en, ru: g.ru, uk: g.uk, parent: g.parent, count: 0,
  }));

  // --- books -------------------------------------------------------
  const books: MockBook[] = [];
  const booksByAuthor = new Map<number, number[]>();
  const booksBySeries = new Map<number, number[]>();
  const bookById = new Map<number, MockBook>();

  function addBook(b: MockBook) {
    books.push(b);
    bookById.set(b.id, b);
    for (const aid of b.authorIds) {
      const arr = booksByAuthor.get(aid) ?? [];
      arr.push(b.id);
      booksByAuthor.set(aid, arr);
    }
    if (b.seriesId) {
      const arr = booksBySeries.get(b.seriesId) ?? [];
      arr.push(b.id);
      booksBySeries.set(b.seriesId, arr);
    }
    for (const gid of b.genreIds) genres[gid - 1].count++;
  }

  let nextId = 1;
  // Arthur Conan Doyle demo bibliography (Sherlock Holmes & Professor Challenger, plus a
  // few standalone historical novels). Names are sorted alphabetically among all 50k
  // authors above, so look ids up by name instead of assuming their pre-sort position.
  const authorIdByName = new Map(authors.map((a) => [a.name, a.id]));
  const seriesIdByName = new Map(series.map((s) => [s.name, s.id]));
  const doyle = [authorIdByName.get('Doyle Arthur Conan')!];
  const holmesSeries = seriesIdByName.get('Sherlock Holmes')!;
  const challengerSeries = seriesIdByName.get('Professor Challenger')!;
  const CLASSIC_MYSTERY = 9;
  const SCI_FI = 2;
  const HISTORICAL_PROSE = 14;
  const demo: [string, number | null, number | null, number, string][] = [
    ['A Study in Scarlet', holmesSeries, 1, CLASSIC_MYSTERY,
      'Dr. Watson meets a brilliant, eccentric detective and is drawn into his first case: a murder investigation that leads back to a story of revenge from the American West.'],
    ['The Sign of the Four', holmesSeries, 2, CLASSIC_MYSTERY,
      'A stolen treasure, a wooden-legged man, and a locked-room death send Holmes and Watson down the Thames in pursuit of a decades-old betrayal.'],
    ['The Adventures of Sherlock Holmes', holmesSeries, 3, CLASSIC_MYSTERY,
      'Twelve of the detective’s early cases, from "A Scandal in Bohemia" to "The Copper Beeches", collected as they first appeared.'],
    ['The Memoirs of Sherlock Holmes', holmesSeries, 4, CLASSIC_MYSTERY,
      'A further eleven cases, ending with the detective’s fateful encounter with Professor Moriarty at the Reichenbach Falls.'],
    ['The Hound of the Baskervilles', holmesSeries, 5, CLASSIC_MYSTERY,
      'A curse, a spectral hound, and a death on the Devon moor call Holmes and Watson to Baskerville Hall.'],
    ['The Return of Sherlock Holmes', holmesSeries, 6, CLASSIC_MYSTERY,
      'Holmes returns from the dead in "The Empty House" and takes on thirteen new cases.'],
    ['The Valley of Fear', holmesSeries, 7, CLASSIC_MYSTERY,
      'A cipher message, a country-house murder, and a secret society lead Holmes to a story that spans continents and decades.'],
    ['His Last Bow', holmesSeries, 8, CLASSIC_MYSTERY,
      'Eight cases, including Holmes’s final wartime mission on the eve of 1914.'],
    ['The Case-Book of Sherlock Holmes', holmesSeries, 9, CLASSIC_MYSTERY,
      'The last twelve recorded cases of the detective, narrated in his later years.'],
    ['The Lost World', challengerSeries, 1, SCI_FI,
      'Professor Challenger leads an expedition to a South American plateau where prehistoric creatures still survive.'],
    ['The Poison Belt', challengerSeries, 2, SCI_FI,
      'Challenger and his companions shut themselves indoors as the Earth passes through a belt of poisonous ether.'],
    ['The Land of Mist', challengerSeries, 3, SCI_FI,
      'Years later, Challenger reluctantly investigates the world of mediums and séances.'],
    ['The White Company', null, null, HISTORICAL_PROSE,
      'A band of English archers and men-at-arms fight through fourteenth-century France and Spain.'],
    ['Sir Nigel', null, null, HISTORICAL_PROSE,
      'A prequel to The White Company, following a young squire’s path to knighthood.'],
    ['Micah Clarke', null, null, HISTORICAL_PROSE,
      'A young soldier’s account of joining the Duke of Monmouth’s doomed rebellion of 1685.'],
  ];
  for (const [title, sid, serno, genre, annotation] of demo) {
    addBook({
      id: nextId, key: `demo:${nextId}`, title, sortKey: normalize(title),
      authorIds: doyle, seriesId: sid, serno, genreIds: [genre],
      lang: 'en', ext: 'fb2', size: int(rand, 200_000, 900_000), date: randDate(rand), deleted: false,
      annotation: `<p>${annotation}</p>`,
    });
    nextId++;
  }

  for (let i = 0; i < BOOK_COUNT; i++) {
    const authorId = int(rand, 1, AUTHOR_COUNT);
    const hasSeries = rand() < 0.55;
    const seriesId = hasSeries ? int(rand, 1, SERIES_COUNT) : null;
    const serno = hasSeries ? int(rand, 1, 9) : null;
    const title = `${pick(rand, TITLE_WORDS_A)} ${pick(rand, TITLE_WORDS_B)}`;
    const genreIds = [int(rand, 1, GENRES.length)];
    if (rand() < 0.3) genreIds.push(int(rand, 1, GENRES.length));
    addBook({
      id: nextId, key: `gen:${nextId}`, title, sortKey: normalize(title),
      authorIds: [authorId], seriesId, serno, genreIds: [...new Set(genreIds)],
      lang: pick(rand, LANGS), ext: pick(rand, EXTS), size: int(rand, 80_000, 2_000_000),
      date: randDate(rand), deleted: rand() < 0.03,
    });
    nextId++;
  }

  for (const b of books) {
    for (const aid of b.authorIds) authors[aid - 1].bookCount++;
    if (b.seriesId) series[b.seriesId - 1].bookCount++;
  }

  return { id, authors, series, genres, books, booksByAuthor, booksBySeries, bookById, authorLetters, seriesLetters };
}
