/**
 * Mirrors the server's normalize(s) from docs/web/ARCHITECTURE.md so client-side
 * filtering agrees with sort_key / letters index ordering:
 * Unicode lower-case, ё→е, й kept, strip punctuation/quotes, collapse whitespace, trim.
 */
const PUNCT = /[«»"'.,:;!?()[\]]/g;

export function normalize(s: string): string {
  return s
    .toLowerCase()
    .replace(/ё/g, 'е')
    .replace(PUNCT, '')
    .replace(/\s+/g, ' ')
    .trim();
}

/** Letter used for the letter index: first char of sort_key, upper-cased, Ё→Е; non-letters → '#'. */
export function letterOf(sortKey: string): string {
  const ch = sortKey.charAt(0).toUpperCase().replace('Ё', 'Е');
  return /[A-ZА-Я]/.test(ch) ? ch : '#';
}
