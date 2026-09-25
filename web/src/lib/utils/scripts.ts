// Scripts of the authors/series letter index: one alphabet is shown at a time.
export type Script = 'cyr' | 'lat' | 'other';

export const ALPHABETS: Record<'cyr' | 'lat', string[]> = {
  cyr: [...'АБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЭЮЯ'],
  lat: [...'ABCDEFGHIJKLMNOPQRSTUVWXYZ'],
};

export const SCRIPT_LABEL: Record<Script, string> = { cyr: 'А–Я', lat: 'A–Z', other: '#' };

const CYRILLIC = /^\p{Script=Cyrillic}/u;
const LATIN = /^\p{Script=Latin}/u;

/** Script of an index letter (`#` = digits, symbols and letters of other scripts). */
export function scriptOf(letter: string): Script {
  if (CYRILLIC.test(letter)) return 'cyr';
  if (LATIN.test(letter)) return 'lat';
  return 'other';
}

export type StripLetter = { letter: string; count: number; index: number | null };

/**
 * The letters shown for `script`: the whole alphabet (letters without names have
 * `index: null` and are shown dimmed), then letters of that script present in the list but
 * not in the basic alphabet (Є, І, Ї, Ґ…).
 */
export function stripLetters(script: Script, letters: [string, number, number][]): StripLetter[] {
  const present = new Map(letters.map((l) => [l[0], l]));
  const out: StripLetter[] = [];
  const base = script === 'other' ? [] : ALPHABETS[script];
  for (const l of base) {
    const p = present.get(l);
    out.push({ letter: l, count: p ? p[1] : 0, index: p ? p[2] : null });
  }
  for (const l of letters) {
    if (scriptOf(l[0]) !== script || base.includes(l[0])) continue;
    out.push({ letter: l[0], count: l[1], index: l[2] });
  }
  if (script === 'other') {
    // `#` last, like its rows
    out.sort((a, b) => (a.letter === '#' ? 1 : 0) - (b.letter === '#' ? 1 : 0));
  }
  return out;
}

/** Scripts with names, most names first. */
export function scriptsByCount(letters: [string, number, number][]): Script[] {
  const n: Record<Script, number> = { cyr: 0, lat: 0, other: 0 };
  for (const l of letters) n[scriptOf(l[0])] += l[1];
  return (['cyr', 'lat', 'other'] as Script[])
    .filter((s) => n[s] > 0)
    .sort((a, b) => (a === 'other' ? 1 : b === 'other' ? -1 : n[b] - n[a]));
}

/** Index of the letter group containing row `i` (letters are in row order). */
export function letterAt(letters: [string, number, number][], i: number): [string, number, number] | null {
  let lo = 0, hi = letters.length - 1, ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (letters[mid][2] <= i) { ans = mid; lo = mid + 1; } else hi = mid - 1;
  }
  return ans >= 0 ? letters[ans] : null;
}
