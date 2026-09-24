import type { Book } from '../api/types';

const ILLEGAL = /[<>:"/\\|?*\u0000-\u001F]/g;

const RU_TO_LAT: Record<string, string> = {
  а: 'a', б: 'b', в: 'v', г: 'g', д: 'd', е: 'e', ё: 'e', ж: 'zh', з: 'z', и: 'i',
  й: 'y', к: 'k', л: 'l', м: 'm', н: 'n', о: 'o', п: 'p', р: 'r', с: 's', т: 't',
  у: 'u', ф: 'f', х: 'h', ц: 'ts', ч: 'ch', ш: 'sh', щ: 'sch', ъ: '', ы: 'y', ь: '',
  э: 'e', ю: 'yu', я: 'ya',
};

function transliterate(s: string): string {
  return s
    .split('')
    .map((ch) => {
      const lower = ch.toLowerCase();
      const t = RU_TO_LAT[lower];
      if (t === undefined) return ch;
      return ch === lower ? t : t.charAt(0).toUpperCase() + t.slice(1);
    })
    .join('');
}

function firstAuthorShort(book: Book): string {
  const a = book.authors[0];
  if (!a) return '';
  const parts = a.name.trim().split(/\s+/);
  const last = parts[0] ?? '';
  const initial = parts[1] ? `${parts[1].charAt(0)}.` : '';
  return [last, initial].filter(Boolean).join(' ');
}

/** File name template placeholders, per docs/web/API.md. */
export function fillFileNameTemplate(template: string, book: Book, opts?: { transliterate?: boolean }): string {
  let out = template
    .replaceAll('%a', firstAuthorShort(book))
    .replaceAll('%fa', book.authors.map((a) => a.name).join(', '))
    .replaceAll('%s', book.series?.name ?? '')
    .replaceAll('%n', book.serno ? String(book.serno).padStart(2, '0') : '')
    .replaceAll('%b', book.title)
    .replaceAll('%l', book.lang)
    .replaceAll('%y', book.date ? book.date.slice(0, 4) : '');
  // Collapse separators left empty by missing parts (" - " -> " ", trim).
  out = out.replace(/\s*-\s*-\s*/g, ' - ').replace(/^[\s-]+|[\s-]+$/g, '').replace(/\s{2,}/g, ' ');
  out = out.replace(ILLEGAL, '');
  if (opts?.transliterate) out = transliterate(out);
  return out;
}
