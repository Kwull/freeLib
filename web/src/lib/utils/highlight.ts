import { normalize } from './normalize';

export type Part = { text: string; hit: boolean };

/**
 * Splits `text` into words and the text between them, marking the words whose normalized form
 * is in `words` (the search response's `highlight`: whole words the server matched by prefix,
 * word form, transliteration or typo fix), so that "книгу" highlights "Книга" and "strugatsky"
 * highlights "Стругацкий".
 */
export function highlightParts(text: string, words: Set<string> | null | undefined): Part[] {
  if (!words || words.size === 0 || !text) return [{ text, hit: false }];
  const out: Part[] = [];
  const re = /[\p{L}\p{N}\p{M}]+/gu;
  let last = 0;
  for (const m of text.matchAll(re)) {
    const i = m.index ?? 0;
    if (i > last) push(out, text.slice(last, i), false);
    push(out, m[0], words.has(normalize(m[0])));
    last = i + m[0].length;
  }
  if (last < text.length) push(out, text.slice(last), false);
  return out;
}

function push(out: Part[], text: string, hit: boolean) {
  const prev = out[out.length - 1];
  // join neighbours with the same state (a highlighted phrase stays one <mark> with its spaces
  // only when both sides are hits)
  if (prev && prev.hit === hit) prev.text += text;
  else out.push({ text, hit });
}
