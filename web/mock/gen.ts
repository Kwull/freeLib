import { mulberry32, pick, int } from './rng';
import { normalize, letterOf } from './normalize';
import {
  SURNAMES, FIRST_NAMES_M, FIRST_NAMES_F, PATRONYMIC_M, PATRONYMIC_F,
  SERIES_WORDS_A, SERIES_WORDS_B, TITLE_WORDS_A, TITLE_WORDS_B, GENRES, LANGS, EXTS,
} from './names';

export type MockAuthor = { id: number; name: string; sortKey: string; bookCount: number };
export type MockSeries = { id: number; name: string; sortKey: string; bookCount: number };
export type MockGenre = { id: number; name: string; parent: number; count: number };
export type MockBook = {
  id: number; key: string; title: string; sortKey: string;
  authorIds: number[]; seriesId: number | null; serno: number | null;
  genreIds: number[]; lang: string; ext: string; size: number; date: string; deleted: boolean;
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
    'Стругацкий Аркадий Натанович', 'Стругацкий Борис Натанович', 'Струве Пётр Бернгардович',
    'Струков Андрей', 'Струн Эдуард', 'Струнский Александр', 'Струтинская Елена',
    'Струтинский Николай', 'Струтт Виктория', 'Струцкий Игорь', 'Струшкевич Анна',
    'Стрюкова Ольга', 'Стрыгин Алексей',
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
  const seedSeries = ['Полдень, XXII век', 'НИИЧАВО', 'Мир Полудня'];
  for (const name of seedSeries) seriesRaw.push({ name, sortKey: normalize(name) });
  for (let i = seedSeries.length; i < SERIES_COUNT; i++) {
    const name = `${pick(rand, SERIES_WORDS_A)} ${pick(rand, SERIES_WORDS_B)}`;
    seriesRaw.push({ name, sortKey: normalize(name) });
  }
  seriesRaw.sort((a, b) => (a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0));
  const series: MockSeries[] = seriesRaw.map((s, i) => ({ id: i + 1, name: s.name, sortKey: s.sortKey, bookCount: 0 }));
  const seriesLetters = buildLetters(series.map((s) => s.sortKey));

  const genres: MockGenre[] = GENRES.map((g, i) => ({ id: i + 1, name: g.name, parent: g.parent, count: 0 }));

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
  // Strugatsky demo bibliography, matching Main.dc.html closely. Names are
  // sorted alphabetically among all 50k authors above, so look ids up by
  // name instead of assuming they kept their pre-sort position.
  const authorIdByName = new Map(authors.map((a) => [a.name, a.id]));
  const seriesIdByName = new Map(series.map((s) => [s.name, s.id]));
  const strug = [authorIdByName.get('Стругацкий Аркадий Натанович')!, authorIdByName.get('Стругацкий Борис Натанович')!];
  const poldenSeries = seriesIdByName.get('Полдень, XXII век')!;
  const niichavoSeries = seriesIdByName.get('НИИЧАВО')!;
  const demo: [string, number | null, number | null, number][] = [
    ['Полдень, XXII век', poldenSeries, 1, 3], ['Попытка к бегству', poldenSeries, 2, 3],
    ['Трудно быть богом', poldenSeries, 3, 2], ['Обитаемый остров', poldenSeries, 5, 1],
    ['Жук в муравейнике', poldenSeries, 6, 1], ['Волны гасят ветер', poldenSeries, 7, 1],
    ['Понедельник начинается в субботу', niichavoSeries, 1, 5], ['Сказка о Тройке', niichavoSeries, 2, 5],
    ['Пикник на обочине', null, null, 2], ['Улитка на склоне', null, null, 11],
    ['Град обреченный', null, null, 11], ['Отель «У погибшего альпиниста»', null, null, 9],
  ];
  for (const [title, sid, serno, genre] of demo) {
    addBook({
      id: nextId, key: `demo:${nextId}`, title, sortKey: normalize(title),
      authorIds: strug, seriesId: sid, serno, genreIds: [genre],
      lang: 'ru', ext: 'fb2', size: int(rand, 200_000, 900_000), date: randDate(rand), deleted: false,
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
