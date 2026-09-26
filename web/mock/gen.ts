import { mulberry32, pick, int } from './rng';
import { normalize, letterOf } from './normalize';
import { indexWorks } from './find';
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
  keywords?: string;
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
  /** `GET authors` / `GET series` rows: names with live books, `#` group last (like the server) */
  authorRows: [number, string, number][];
  seriesRows: [number, string, number][];
  authorLetters: [string, number, number][];
  seriesLetters: [string, number, number][];
};

/** A book with this many authors is an anthology (same rule as the server). */
export const ANTHOLOGY_MIN_AUTHORS = 4;

const AUTHOR_COUNT = 50_000;
const SERIES_COUNT = 4_000;
const BOOK_COUNT = 3_000;

/**
 * Rows of a name list the way the server sends them: names with live books only, in sort-key
 * order with the non-letter (`#`) group moved to the end, and the letter index over them.
 */
function nameList(items: { id: number; name: string; sortKey: string; bookCount: number }[]) {
  const letters: [string, number, number][] = [];
  const rows: [number, string, number][] = [];
  const other: [number, string, number][] = [];
  for (const it of items) {
    if (it.bookCount <= 0) continue;
    const l = letterOf(it.sortKey);
    if (l === '#') { other.push([it.id, it.name, it.bookCount]); continue; }
    const last = letters[letters.length - 1];
    if (last && last[0] === l) last[1]++;
    else letters.push([l, 1, rows.length]);
    rows.push([it.id, it.name, it.bookCount]);
  }
  if (other.length) { letters.push(['#', other.length, rows.length]); rows.push(...other); }
  return { rows, letters };
}

function randDate(rand: () => number): string {
  const start = new Date('2022-01-01').getTime();
  const end = new Date('2026-09-24').getTime();
  const d = new Date(start + rand() * (end - start));
  return d.toISOString().slice(0, 10);
}

const ASIMOV_SERIES = [
  'Foundation', 'Robots', 'Galactic Empire', 'Lucky Starr', 'The Black Widowers', 'Norby', 'Wendell Urth',
  'Isaac Asimov Presents: The Great SF Stories', 'Masters of Science Fiction (Gollancz)',
  'The Complete Stories', 'Robot City', 'Azazel', 'Collected Fiction in Twelve Volumes (Doubleday, 1968–1990)',
  'Isaac Asimov Science Fiction Magazine Collections', 'Nightfall and Other Stories',
];
const ANTHOLOGY_SERIES = [
  'The Best of the Year: Science Fiction Anthology (Great Masters of the Golden Age, Gollancz 1970–1995)',
  'The Hugo Winners', 'Wonder Stories Anthology', 'Year\'s Best SF',
];
const ASIMOV_TITLES = [
  'I, Robot', 'The End of Eternity', 'Foundation', 'Foundation and Empire', 'Second Foundation', 'The Gods Themselves',
  'The Caves of Steel', 'The Naked Sun', 'The Robots of Dawn', 'The Bicentennial Man', 'Nightfall', 'Nemesis',
  'Pebble in the Sky', 'Fantastic Voyage', 'The Positronic Man', 'The Ugly Little Boy',
];
const ANTHOLOGY_TITLES = [
  'The Great SF Stories', 'The Hugo Winners, Volume', 'Year\'s Best SF', 'Before the Golden Age', 'Space Mail',
  'Tomorrow\'s Children', 'Robots and Aliens', 'The Science Fiction Hall of Fame',
];

export function generateLibrary(id: number, seed: number): MockLibrary {
  const rand = mulberry32(seed);

  // --- authors -------------------------------------------------------
  const authorsRaw: { name: string; sortKey: string }[] = [];
  // A guaranteed, prototype-like cluster of authors so demos/tests are stable.
  const seedAuthors = [
    'Doyle Arthur Conan', 'Doyley Arthur', 'Doylan Frederick', 'Doyler Henry',
    'Doyleston Margaret', 'Doyne Robert', 'Doynton Grace', 'Doyce Samuel',
    // a prolific, anthology-heavy author (like Asimov on Flibusta: ~1000 books, ~100 series,
    // thousands of names sharing an anthology with him) and his real co-author
    'Asimov Isaac', 'Silverberg Robert',
    // accented Latin (folded to A–Z in the letter index), Cyrillic, and non-letter names
    'Čapek Karel', 'Lem Stanisław', 'Ødegaard Knut', 'Азимов Айзек', 'Стругацкий Аркадий Натанович',
    '1984 Group', '4PDA Team',
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

  // --- series ----------------------------------------------------------
  const seriesRaw: { name: string; sortKey: string }[] = [];
  const seedSeries = ['Sherlock Holmes', 'Professor Challenger', ...ASIMOV_SERIES, ...ANTHOLOGY_SERIES];
  for (const name of seedSeries) seriesRaw.push({ name, sortKey: normalize(name) });
  for (let i = seedSeries.length; i < SERIES_COUNT; i++) {
    const name = `${pick(rand, SERIES_WORDS_A)} ${pick(rand, SERIES_WORDS_B)}`;
    seriesRaw.push({ name, sortKey: normalize(name) });
  }
  seriesRaw.sort((a, b) => (a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0));
  const series: MockSeries[] = seriesRaw.map((s, i) => ({ id: i + 1, name: s.name, sortKey: s.sortKey, bookCount: 0 }));

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

  // --- the prolific author: own books in many series, co-written books, anthologies ------
  const asimov = authorIdByName.get('Asimov Isaac')!;
  const silverberg = authorIdByName.get('Silverberg Robert')!;
  const asimovSeries = ASIMOV_SERIES.map((n) => seriesIdByName.get(n)!);
  const anthologySeries = ANTHOLOGY_SERIES.map((n) => seriesIdByName.get(n)!);
  for (let i = 0; i < 180; i++) {
    const inSeries = rand() < 0.7;
    const title = i < ASIMOV_TITLES.length ? ASIMOV_TITLES[i] : `${pick(rand, TITLE_WORDS_A)} ${pick(rand, TITLE_WORDS_B)}`;
    addBook({
      id: nextId, key: `asimov:${nextId}`, title, sortKey: normalize(title),
      authorIds: i % 45 === 7 ? [asimov, silverberg] : [asimov],
      seriesId: inSeries ? asimovSeries[Math.floor(Math.pow(rand(), 1.6) * asimovSeries.length)] : null,
      serno: inSeries ? int(rand, 1, 12) : null,
      genreIds: [pick(rand, [SCI_FI, 3, 4]), ...(rand() < 0.3 ? [22] : [])],
      lang: rand() < 0.9 ? 'en' : 'ru', ext: 'fb2', size: int(rand, 150_000, 1_500_000), date: randDate(rand), deleted: rand() < 0.03,
    });
    nextId++;
  }
  for (let i = 0; i < 150; i++) {
    const n = 5 + Math.floor(25 * Math.pow(rand(), 2));
    const ids = new Set<number>([asimov]);
    if (rand() < 0.15) ids.add(silverberg);
    while (ids.size < n) ids.add(int(rand, 1, AUTHOR_COUNT));
    const order = [...ids].sort(() => rand() - 0.5);
    const title = `${pick(rand, ANTHOLOGY_TITLES)} ${int(rand, 1, 40)}`;
    const inSeries = rand() < 0.6;
    addBook({
      id: nextId, key: `anth:${nextId}`, title, sortKey: normalize(title),
      authorIds: order, seriesId: inSeries ? pick(rand, anthologySeries) : null, serno: inSeries ? int(rand, 1, 60) : null,
      genreIds: [SCI_FI], lang: 'en', ext: 'fb2', size: int(rand, 400_000, 3_000_000), date: randDate(rand), deleted: rand() < 0.03,
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

  // Every author has at least one live book (the server lists only those): one standalone
  // book each for the rest of the 50k names.
  const hasLive = new Set<number>();
  for (const b of books) if (!b.deleted) for (const a of b.authorIds) hasLive.add(a);
  for (const a of authors) {
    if (hasLive.has(a.id)) continue;
    const title = `${pick(rand, TITLE_WORDS_A)} ${pick(rand, TITLE_WORDS_B)}`;
    const hasSeries = rand() < 0.3;
    addBook({
      id: nextId, key: `fill:${nextId}`, title, sortKey: normalize(title),
      authorIds: [a.id], seriesId: hasSeries ? int(rand, 1, SERIES_COUNT) : null, serno: hasSeries ? int(rand, 1, 9) : null,
      genreIds: [int(rand, 1, GENRES.length)], lang: pick(rand, LANGS), ext: pick(rand, EXTS),
      size: int(rand, 80_000, 2_000_000), date: randDate(rand), deleted: false,
    });
    nextId++;
  }

  // --- Russian books for the start page, word-form / transliteration search and editions ---
  // (appended after everything random, so the rest of the library stays as it was)
  const byKey = (a: { sortKey: string }, b: { sortKey: string }) => (a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0);
  const addAuthor = (name: string) => {
    const a: MockAuthor = { id: authors.length + 1, name, sortKey: normalize(name), bookCount: 0 };
    authors.push(a);
    return a.id;
  };
  const addSeries = (name: string) => {
    const s: MockSeries = { id: series.length + 1, name, sortKey: normalize(name), bookCount: 0 };
    series.push(s);
    return s.id;
  };
  const arkady = authorIdByName.get('Стругацкий Аркадий Натанович')!;
  const boris = addAuthor('Стругацкий Борис Натанович');
  const azimov = authorIdByName.get('Азимов Айзек')!;
  const noon = addSeries('Мир Полудня');
  const robots = addSeries('Роботы');
  const strug = [arkady, boris];
  const RU_SF = 2;
  const curated: [string, number[], number | null, number | null, string, number, string, string?][] = [
    ['Полдень, XXII век', strug, noon, 1, 'fb2', 820_000, '2024-03-01'],
    ['Попытка к бегству', strug, noon, 2, 'fb2', 410_000, '2024-05-01'],
    ['Трудно быть богом', strug, noon, 3, 'fb2', 640_000, '2025-01-10'],
    ['Обитаемый остров', strug, noon, 4, 'fb2', 1_100_000, '2026-09-10'],
    ['Жук в муравейнике', strug, noon, 5, 'fb2', 520_000, '2026-09-15'],
    ['Пикник на обочине', strug, null, null, 'fb2', 700_000, '2023-01-01'],
    ['Пикник на обочине (другой перевод)', strug, null, null, 'epub', 900_000, '2024-02-01', 'перевод А. Бромфилда'],
    ['Пикник на обочине', strug, null, null, 'fb2', 1_250_000, '2022-06-01'],
    ['Пикник на обочине [иллюстрации]', strug, null, null, 'fb2', 400_000, '2025-02-01'],
    ['Град обреченный', strug, null, null, 'fb2', 980_000, '2026-09-18'],
    ['Я, робот', [azimov], robots, 1, 'fb2', 560_000, '2023-04-01'],
    ['Я, робот (другой перевод)', [azimov], robots, 1, 'fb2', 590_000, '2024-04-01'],
    ['Стальные пещеры', [azimov], robots, 2, 'fb2', 610_000, '2025-06-01'],
    ['Обнажённое солнце', [azimov], robots, 3, 'fb2', 600_000, '2026-09-05'],
    ['Конец Вечности', [azimov], null, null, 'fb2', 700_000, '2026-09-20'],
    ['Книга о книгах', [azimov], null, null, 'fb2', 300_000, '2026-09-21'],
    ['Записки на полях', [azimov], null, null, 'fb2', 200_000, '2021-01-01', 'азимвв'],
  ];
  // Asimov's Foundation novels in several Russian translations under different titles and
  // numbers (as in a real Flibusta library), unnumbered omnibus volumes, and a two-volume
  // novel: the importer's edition rules (docs/web/ARCHITECTURE.md, "Editions").
  const academy = addSeries('Академия [Азимов]');
  const marinina = addAuthor('Маринина Александра');
  const kamenskaya = addSeries('Каменская');
  curated.push(
    ['Прелюдия к Академии', [azimov], academy, 1, 'fb2', 764_000, '2018-07-09'],
    ['Прелюдия к Основанию', [azimov], academy, 1, 'fb2', 429_000, '2012-11-15'],
    ['Миры Айзека Азимова. Книга 5', [azimov], academy, 1, 'fb2', 2_900_000, '2010-02-02'],
    ['Академия', [azimov], academy, 3, 'fb2', 380_000, '2016-03-01'],
    ['Основание', [azimov], academy, 3, 'fb2', 402_000, '2011-05-01'],
    ['Основание (другой перевод)', [azimov], academy, 3, 'epub', 450_000, '2019-05-01', 'перевод Н. Сосновской'],
    ['Установление', [azimov], academy, 3, 'fb2', 350_000, '2009-01-20'],
    ['Фонд', [azimov], academy, 2, 'fb2', 390_000, '2015-01-01'],
    ['Фонд [litres]', [azimov], academy, 2, 'fb2', 395_000, '2021-01-01'],
    ['Второй Фонд', [azimov], academy, 5, 'fb2', 420_000, '2014-04-04'],
    ['Дублеры', [azimov], academy, 5, 'fb2', 410_000, '2008-08-08'],
    ['Академия на краю гибели', [azimov], academy, 6, 'fb2', 610_000, '2017-06-01'],
    ['Академия на краю гибели', [azimov], academy, 6, 'fb2', 598_000, '2013-06-01'],
    ['Академия на краю гибели (fb2)', [azimov], academy, 6, 'fb2', 605_000, '2020-06-01'],
    ['Край Основания', [azimov], academy, 6, 'fb2', 640_000, '2012-02-01'],
    ['Миры Айзека Азимова. Книга 9', [azimov], academy, 6, 'fb2', 3_100_000, '2010-03-03'],
    ['Сообщество на краю', [azimov], academy, 6, 'fb2', 590_000, '2007-09-09'],
    ['Академия и Земля', [azimov], academy, 7, 'fb2', 700_000, '2017-07-01'],
    ['Академия и Земля', [azimov], academy, 7, 'epub', 720_000, '2022-07-01'],
    ['Миры Айзека Азимова. Книга 10', [azimov], academy, 7, 'fb2', 3_000_000, '2010-04-04'],
    ['Основание и Земля', [azimov], academy, 7, 'fb2', 690_000, '2012-12-12'],
    ['Сообщество и Земля', [azimov], academy, 7, 'fb2', 680_000, '2007-10-10'],
    ['Страхи Академии', [azimov], academy, 8, 'fb2', 500_000, '2016-01-15'],
    ['Академия и Хаос', [azimov], academy, 9, 'fb2', 520_000, '2016-02-15'],
    ['Триумф Академии', [azimov], academy, 10, 'fb2', 530_000, '2016-03-15'],
    ['Академия. Книги 1-7', [azimov], academy, null, 'fb2', 4_800_000, '2019-09-09'],
    ['Академия. Начало', [azimov], academy, null, 'fb2', 1_200_000, '2020-01-01'],
    ['Академия. Первая трилогия', [azimov], academy, null, 'fb2', 1_900_000, '2018-01-01'],
    ['Миры Айзека Азимова. Книга 7', [azimov], academy, null, 'fb2', 2_800_000, '2010-05-05'],
    ['Путь к Академии', [azimov], academy, null, 'fb2', 300_000, '2021-05-05'],
    ['Люди за спиной. Том 1', [marinina], kamenskaya, 37, 'fb2', 820_000, '2024-01-10'],
    ['Люди за спиной, том 1', [marinina], kamenskaya, 37, 'fb2', 810_000, '2023-11-10'],
    ['Люди за спиной. Том 2', [marinina], kamenskaya, 37, 'fb2', 790_000, '2024-02-10'],
  );
  for (const [title, aids, sid, serno, ext, size, date, keywords] of curated) {
    addBook({
      id: nextId, key: `ru:${nextId}`, title, sortKey: normalize(title),
      authorIds: aids, seriesId: sid, serno, genreIds: [RU_SF], lang: 'ru', ext, size, date, deleted: false, keywords,
    });
    nextId++;
  }

  for (const b of books) {
    if (b.deleted) continue;
    for (const aid of b.authorIds) authors[aid - 1].bookCount++;
    if (b.seriesId) series[b.seriesId - 1].bookCount++;
  }
  const al = nameList(authors.slice().sort(byKey));
  const sl = nameList(series.slice().sort(byKey));

  const lib: MockLibrary = {
    id, authors, series, genres, books, booksByAuthor, booksBySeries, bookById,
    authorRows: al.rows, seriesRows: sl.rows, authorLetters: al.letters, seriesLetters: sl.letters,
  };
  indexWorks(lib);
  return lib;
}
