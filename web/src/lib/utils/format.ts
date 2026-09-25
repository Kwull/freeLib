export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatDate(iso: string, locale = 'ru'): string {
  if (!iso) return '';
  const [y, m, d] = iso.split('-');
  if (!y) return iso;
  return locale === 'ru' ? `${d}.${m}.${y}` : `${y}-${m}-${d}`;
}

export function initialsBg(seed: string): string {
  const palette = ['#2F4A5A', '#6B3A2E', '#3E5A3A', '#4A4063', '#5A4A2F', '#3A4A5C', '#5A3A4A'];
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return palette[h % palette.length];
}

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number): (...a: A) => void {
  let t: ReturnType<typeof setTimeout> | undefined;
  return (...a: A) => {
    if (t) clearTimeout(t);
    t = setTimeout(() => fn(...a), ms);
  };
}
