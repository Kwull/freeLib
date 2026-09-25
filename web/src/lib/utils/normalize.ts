/**
 * Exact port of the server's `normalize` / `letter_of`
 * (server/crates/catalog/src/normalize.rs), so client-side filtering agrees with the
 * sort_key / letter index order. Both implementations are checked against the shared
 * vectors in docs/web/normalize-vectors.json (web/tests-unit/normalize.test.ts and
 * server/crates/catalog/tests/normalize_vectors.rs) — change them together.
 *
 * Rust → JS mapping: `char::is_whitespace` = \p{White_Space}, `is_control` = \p{Cc},
 * `is_alphanumeric` = \p{Alphabetic} or \p{N}, `char::to_lowercase` = per-code-point
 * `toLowerCase()` (no context-dependent final sigma).
 */

/** Dropped entirely (quotes, apostrophes): `O'Brien` → `obrien`. */
const DROPPED = new Set(["'", '"', '`', '«', '»', '„', '“', '”', '‘', '’', '‚', '‹', '›', '´']);
/** Word separators besides whitespace and control characters: `Толстой Л.Н.` → `толстой л н`. */
const SEPARATORS = new Set(['.', ',', ':', ';', '!', '?', '(', ')', '[', ']', '{', '}', '…', '—', '–', '/', '\\', '|']);
const WHITE_SPACE = /^\p{White_Space}$/u;
const CONTROL = /^\p{Cc}$/u;
const ALPHANUMERIC = /^[\p{Alphabetic}\p{N}]$/u;
const ALPHABETIC = /^\p{Alphabetic}$/u;

/** 0 = dropped, 1 = separator, 2 = alphanumeric, 3 = other. */
function classify(c: string, code: number): number {
  // fast paths for ASCII and basic Cyrillic (almost all catalog names)
  if (code < 0x80) {
    if (code === 0x22 || code === 0x27 || code === 0x60) return 0;
    if (code <= 0x20 || code === 0x7f) return 1;
    if ((code >= 0x30 && code <= 0x39) || (code >= 0x41 && code <= 0x5a) || (code >= 0x61 && code <= 0x7a)) return 2;
    return SEPARATORS.has(c) ? 1 : 3;
  }
  if ((code >= 0x410 && code <= 0x44f) || code === 0x401 || code === 0x451) return 2;
  if (DROPPED.has(c)) return 0;
  if (SEPARATORS.has(c) || WHITE_SPACE.test(c) || CONTROL.test(c)) return 1;
  return ALPHANUMERIC.test(c) ? 2 : 3;
}

function lower(c: string, code: number): string {
  if (code >= 0x41 && code <= 0x5a) return String.fromCharCode(code + 32);
  if (code < 0x80) return c;
  if (code >= 0x410 && code <= 0x42f) return String.fromCharCode(code + 32);
  if (code === 0x401 || code === 0x451) return 'е';
  if (code >= 0x430 && code <= 0x44f) return c;
  let out = '';
  for (const lc of c.toLowerCase()) out += lc === 'ё' ? 'е' : lc;
  return out;
}

export function normalize(s: string): string {
  let out = '';
  let pendingSpace = false;
  for (let i = 0; i < s.length; ) {
    const code = s.codePointAt(i)!;
    const c = code > 0xffff ? String.fromCodePoint(code) : s[i];
    i += c.length;
    const kind = classify(c, code);
    if (kind === 0) continue;
    if (kind === 1) {
      pendingSpace = true;
      continue;
    }
    // skip leading dashes, asterisks, etc.
    if (out === '' && kind !== 2) continue;
    if (pendingSpace && out !== '') out += ' ';
    pendingSpace = false;
    out += lower(c, code);
  }
  if (out === '') {
    // only punctuation: trimmed, lower-cased, whitespace collapsed
    return s
      .toLowerCase()
      .split(/\p{White_Space}+/u)
      .filter((w) => w !== '')
      .join(' ');
  }
  return out;
}

/** Index letter of a sort key: first character upper-cased when it is a letter (Ё → Е), `#` otherwise. */
export function letterOf(sortKey: string): string {
  const first = sortKey.codePointAt(0);
  if (first === undefined) return '#';
  const c = String.fromCodePoint(first);
  if (!ALPHABETIC.test(c)) return '#';
  const up = c.toUpperCase();
  return up === 'Ё' ? 'Е' : up;
}
