/**
 * Port of the server's `phonetic_key` and `name_rank` (server/crates/catalog/src/text.rs and
 * search.rs): the coarse Latin key that lets `asimov`, `azimov`, `asimow` and `azimoff` find
 * `Азимов`, and the order of names in search results and the name filter. Both
 * implementations are checked against docs/web/phonetic-vectors.json
 * (web/tests-unit/phonetic.test.ts and server/crates/catalog/tests/phonetic_vectors.rs);
 * change them together.
 */
const TR: Record<string, string> = {
  а: 'a', б: 'b', в: 'v', г: 'g', ґ: 'g', д: 'd', е: 'e', ё: 'e', є: 'ye', ж: 'zh', з: 'z', и: 'i', і: 'i', ї: 'yi',
  й: 'y', к: 'k', л: 'l', м: 'm', н: 'n', о: 'o', п: 'p', р: 'r', с: 's', т: 't', у: 'u', ф: 'f', х: 'kh', ц: 'ts',
  ч: 'ch', ш: 'sh', щ: 'shch', ъ: '', ь: '', ы: 'y', э: 'e', ю: 'yu', я: 'ya',
};

/** Russian / Ukrainian → Latin (the server's `translit`), lower case. */
export function translit(s: string): string {
  let out = '';
  for (const c of s) {
    const l = c.toLowerCase();
    const t = TR[l];
    out += t === undefined ? c : t;
  }
  return out;
}

const VOWELS = new Set(['a', 'e', 'i', 'o', 'u']);
const ASCII_LETTER = /^[a-z]$/;

/** Coarse phonetic key of one *normalized* word (see `phonetic_key` in text.rs for the rules). */
export function phoneticKey(w: string): string {
  let s = translit(w).toLowerCase().replace(/[^a-z0-9]/g, '').replaceAll('w', 'v').replaceAll('q', 'k');
  for (const [from, to] of [
    ['tsch', 'q'], ['shch', 'w'], ['sch', 'w'], ['sh', 'w'], ['zh', 'w'], ['tch', 'q'], ['ch', 'q'],
    ['kh', 'h'], ['ph', 'f'], ['ks', 'x'], ['ts', 'c'], ['tz', 'c'], ['tc', 'c'], ['z', 's'],
  ] as const) s = s.replaceAll(from, to);
  let t = '';
  for (let i = 0; i < s.length; ) {
    const c = s[i];
    const prev = i > 0 ? s[i - 1] : '';
    if ((c === 'y' || c === 'j') && s[i + 1] === 'o' && i > 0 && ASCII_LETTER.test(prev) && !VOWELS.has(prev) && prev !== 'y') {
      t += 'e';
      i += 2;
      continue;
    }
    t += c === 'y' || c === 'j' ? 'i' : c;
    i++;
  }
  t = t.replaceAll('ie', 'e');
  let out = '';
  for (const c of t) {
    if (out.endsWith(c) && ASCII_LETTER.test(c)) continue;
    out += c;
  }
  if (out.length > 3) {
    if (out.endsWith('of')) out = `${out.slice(0, -2)}ov`;
    else if (out.endsWith('ef')) out = `${out.slice(0, -2)}ev`;
  }
  return out;
}

const WORD_SPLIT = /[^\p{L}\p{N}]+/u;
const HAS_LETTER = /\p{L}/u;

/** Words of normalized text (runs of letters and digits, like the server's FTS tokenizer). */
export function wordsOf(normalized: string): string[] {
  return normalized.split(WORD_SPLIT).filter(Boolean);
}

/** The search key of a normalized word (`latin_key`): its phonetic key for words of at least
 *  3 characters with a letter, else null. */
export function latinKey(w: string): string | null {
  if ([...w].length < 3 || !HAS_LETTER.test(w)) return null;
  const k = phoneticKey(w);
  return k.length >= 2 ? k : null;
}

/**
 * How well a normalized name matches normalized query words, best first: 0 = its first word
 * (an author's last name) is a query word, as typed or by phonetic key; 1 = the first word
 * starts with one; 2 = another word is one; 3 = another word starts with one (or no match).
 */
export function nameRank(name: string, tokens: string[]): number {
  const toks = tokens.map((t) => ({ t, k: latinKey(t), long: [...t].length >= 3 }));
  let best = 3;
  const ws = wordsOf(name);
  for (let j = 0; j < ws.length && best > 0; j++) {
    const w = ws[j];
    const wk = latinKey(w);
    for (const { t, k, long } of toks) {
      const whole = w === t || (k !== null && wk === k);
      const prefix = w.startsWith(t) || (long && k !== null && wk !== null && wk.startsWith(k));
      const r = j === 0 ? (whole ? 0 : prefix ? 1 : 3) : whole ? 2 : 3;
      if (r < best) best = r;
    }
  }
  return best;
}
