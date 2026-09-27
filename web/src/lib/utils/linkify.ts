// http(s) URLs in plain text become links: finding them (no DOM here, unit-tested with node).
// linkifyHtml.ts builds the anchors.

const URL_RE = /\bhttps?:\/\/[^\s<>"'«»“”]+/giu;
const TRAILING_CHARS = '.,;:!?)]}\'"»”';

export const LINK_REL = 'noopener noreferrer nofollow';

/** The URL at the start of `raw` without trailing punctuation, or null when it is not http(s).
 *  A closing parenthesis stays when the URL opened one ("…/Foo_(bar)"). */
export function cleanUrl(raw: string): string | null {
  let u = raw;
  const count = (c: string) => u.split(c).length - 1;
  while (u.length) {
    const last = u[u.length - 1];
    if (!TRAILING_CHARS.includes(last)) break;
    if (last === ')' && count('(') >= count(')')) break;
    u = u.slice(0, -1);
  }
  try {
    const parsed = new URL(u);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
  } catch {
    return null;
  }
  return u;
}

/** Splits a text into plain strings and URLs. */
export function splitUrls(text: string): { text: string; url?: string }[] {
  const out: { text: string; url?: string }[] = [];
  let last = 0;
  for (const m of text.matchAll(URL_RE)) {
    const url = cleanUrl(m[0]);
    if (!url) continue;
    const at = m.index ?? 0;
    if (at > last) out.push({ text: text.slice(last, at) });
    out.push({ text: url, url });
    last = at + url.length;
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}
