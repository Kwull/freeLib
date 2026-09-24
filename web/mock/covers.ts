function bgFor(seed: string): string {
  const palette = ['2F4A5A', '6B3A2E', '3E5A3A', '4A4063', '5A4A2F', '3A4A5C', '5A3A4A'];
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return palette[h % palette.length];
}

function esc(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function wrap(title: string, max: number): string[] {
  const words = title.split(' ');
  const lines: string[] = [];
  let cur = '';
  for (const w of words) {
    const next = cur ? `${cur} ${w}` : w;
    if (next.length > max && cur) { lines.push(cur); cur = w; } else cur = next;
    if (lines.length === 3) break;
  }
  if (cur && lines.length < 3) lines.push(cur);
  return lines;
}

/** Placeholder cover SVG generated from the title, like the prototype's colored blocks. */
export function placeholderCover(title: string, size: 'thumb' | 'full'): string {
  const w = size === 'thumb' ? 160 : 320;
  const h = size === 'thumb' ? 240 : 480;
  const bg = bgFor(title);
  const lines = wrap(title, size === 'thumb' ? 12 : 18);
  const fs = size === 'thumb' ? 13 : 22;
  const text = lines
    .map((l, i) => `<text x="16" y="${h - 24 - (lines.length - 1 - i) * (fs + 6)}" font-family="Georgia, serif" font-size="${fs}" fill="#FFFFFF">${esc(l)}</text>`)
    .join('');
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">
<rect width="${w}" height="${h}" fill="#${bg}"/>
${text}
</svg>`;
}
