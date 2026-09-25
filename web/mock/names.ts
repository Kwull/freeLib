// English name pools. Author display order is "Surname First Middle" (no comma), matching
// how the real catalog sorts by surname first letter.
export const SURNAMES = [
  'Smith', 'Johnson', 'Williams', 'Brown', 'Jones', 'Doyle', 'Miller', 'Davis', 'Wilson', 'Taylor',
  'Moore', 'Clark', 'Turner', 'Baker', 'Carter', 'Mitchell', 'Foster', 'Bennett', 'Reed', 'Hayes',
  'Cooper', 'Bailey', 'Morgan', 'Murphy', 'Bell', 'Coleman', 'Hunter', 'Fisher', 'Ferguson', 'Knight',
  'Dawson', 'Barnes', 'Hughes', 'Palmer', 'Chapman', 'Conway', 'Rutherford', 'Ashford', 'Whitfield', 'Sinclair',
  'Dickens', 'Austen', 'Bronte', 'Wells', 'Stoker', 'Christie', 'Kipling', 'Stevenson', 'Wilde', 'Shelley',
  'Verne', 'Wodehouse', 'Chesterton', 'Sayers', 'Sabatini', 'Buchan', 'Haggard', 'Hope', 'Marsh', 'Allingham',
];
export const FIRST_NAMES_M = [
  'Arthur', 'James', 'John', 'Robert', 'William', 'Charles', 'Edward', 'George', 'Henry', 'Thomas',
  'Frederick', 'Albert', 'Walter', 'Samuel', 'Joseph', 'Francis', 'Herbert', 'Alfred', 'Ernest', 'Sidney',
];
export const FIRST_NAMES_F = [
  'Mary', 'Elizabeth', 'Margaret', 'Alice', 'Emily', 'Florence', 'Edith', 'Grace', 'Helen', 'Jane',
  'Catherine', 'Eleanor', 'Charlotte', 'Louisa', 'Beatrice',
];
// Note: 'Conan' is deliberately excluded so the seeded "Doyle Arthur Conan"
// demo author can never collide with a randomly generated middle name.
export const PATRONYMIC_M = [
  'Edward', 'Henry', 'James', 'Robert', 'William', 'Charles', 'Frederick', 'George',
  'Albert', 'Thomas', 'Francis', 'Herbert', 'Walter', 'Ernest',
];
export const PATRONYMIC_F = [
  'Anne', 'Jane', 'Louise', 'Grace', 'Elizabeth', 'Margaret', 'Rose', 'Kate',
  'Catherine', 'Florence', 'Alice', 'Mary', 'Eleanor', 'Edith',
];

export const SERIES_WORDS_A = [
  'World', 'Chronicles', 'Legends', 'Tales', 'Empire', 'Stars', 'Meridian', 'Shadows', 'Realm', 'Road',
  'Legacy', 'Blades', 'Flame', 'Wind', 'Gates', 'Fragments', 'Kingdom', 'Light', 'Dark', 'Time',
];
export const SERIES_WORDS_B = [
  'of Oblivion', 'of the Gods', 'of the Stars', 'of Dreams', 'of the Void', 'of the Empire', 'of Wanderers',
  'of Heroes', 'of War', 'of Ice', 'of Fire', 'of the Wind', 'of Shadows', 'of Time', 'of Fate',
  'of Hope', 'of Night', 'of Dawn', 'of the Abyss', 'of the Horizon',
];

export const TITLE_WORDS_A = [
  'The Last', 'The Secret', 'The Forgotten', 'The Great', 'The Strange', 'The Dark', 'The White', 'The Iron',
  'The Burning', 'The Frozen', 'The Morning', 'The Midnight', 'The Endless', 'The Missing', 'The Cursed',
  'The Golden', 'The Silver', 'The Lost', 'The New', 'The Ancient',
];
export const TITLE_WORDS_B = [
  'City', 'Road', 'Island', 'House', 'Ship', 'Forest', 'Garden', 'Portal', 'World', 'Temple',
  'Blade', 'Key', 'Diary', 'Bargain', 'Flight', 'Sunrise', 'Sunset', 'Shore', 'Harbor', 'Horizon',
];

export const GENRES: { en: string; ru: string; uk: string; parent: number }[] = [
  { en: 'Fiction', ru: 'Фантастика', uk: 'Фантастика', parent: 0 },
  { en: 'Science Fiction', ru: 'Научная фантастика', uk: 'Наукова фантастика', parent: 1 },
  { en: 'Social Sci-Fi', ru: 'Социальная фантастика', uk: 'Соціальна фантастика', parent: 1 },
  { en: 'Space Opera Sci-Fi', ru: 'Космическая фантастика', uk: 'Космічна фантастика', parent: 1 },
  { en: 'Fantasy', ru: 'Фэнтези', uk: 'Фентезі', parent: 0 },
  { en: 'Heroic Fantasy', ru: 'Героическое фэнтези', uk: 'Героїчне фентезі', parent: 5 },
  { en: 'Urban Fantasy', ru: 'Городское фэнтези', uk: 'Міське фентезі', parent: 5 },
  { en: 'Mystery & Thrillers', ru: 'Детективы и триллеры', uk: 'Детективи та трилери', parent: 0 },
  { en: 'Classic Mystery', ru: 'Классический детектив', uk: 'Класичний детектив', parent: 8 },
  { en: 'Thriller', ru: 'Триллер', uk: 'Трилер', parent: 8 },
  { en: 'Prose', ru: 'Проза', uk: 'Проза', parent: 0 },
  { en: 'Contemporary Prose', ru: 'Современная проза', uk: 'Сучасна проза', parent: 11 },
  { en: 'Classic Prose', ru: 'Классическая проза', uk: 'Класична проза', parent: 11 },
  { en: 'Historical Prose', ru: 'Историческая проза', uk: 'Історична проза', parent: 11 },
  { en: 'Journalism', ru: 'Публицистика', uk: 'Публіцистика', parent: 0 },
  { en: 'Drama', ru: 'Драматургия', uk: 'Драматургія', parent: 0 },
  { en: 'Poetry', ru: 'Поэзия', uk: 'Поезія', parent: 0 },
  { en: "Children's Literature", ru: 'Детская литература', uk: 'Дитяча література', parent: 0 },
  { en: "Children's Prose", ru: 'Детская проза', uk: 'Дитяча проза', parent: 18 },
  { en: "Children's Adventure", ru: 'Детские приключения', uk: 'Дитячі пригоди', parent: 18 },
  { en: 'Science & Education', ru: 'Наука и образование', uk: 'Наука і освіта', parent: 0 },
  { en: 'History', ru: 'История', uk: 'Історія', parent: 21 },
  { en: 'Philosophy', ru: 'Философия', uk: 'Філософія', parent: 21 },
  { en: 'Psychology', ru: 'Психология', uk: 'Психологія', parent: 21 },
  { en: 'Other', ru: 'Прочее', uk: 'Інше', parent: 0 },
];

export function genreName(g: { en: string; ru: string; uk: string }, lang: string): string {
  return lang === 'ru' ? g.ru : lang === 'uk' ? g.uk : g.en;
}

export const LANGS = ['en', 'ru', 'uk'] as const;
export const EXTS = ['fb2', 'epub', 'pdf'] as const;
