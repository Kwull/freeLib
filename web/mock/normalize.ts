const PUNCT = /[«»"'.,:;!?()[\]]/g;

export function normalize(s: string): string {
  return s.toLowerCase().replace(/ё/g, 'е').replace(PUNCT, '').replace(/\s+/g, ' ').trim();
}

export function letterOf(sortKey: string): string {
  const ch = sortKey.charAt(0).toUpperCase().replace('Ё', 'Е');
  return /[A-ZА-Я]/.test(ch) ? ch : '#';
}
