// The search page's state in the URL query: `q`, `exact=1`, the facet filters and the sort, so
// Back/Forward and shared links keep them. Unknown parameters are left alone.
import { emptyRatingFilters, type RatingFilters, type RatingSortKey } from './ratings';

export type SearchSort = 'relevance' | RatingSortKey;
const SORTS: SearchSort[] = ['relevance', 'myRating', 'libRating', 'extRating'];

export type SearchFilters = {
  genre: Set<number>;
  lang: Set<string>;
  ext: Set<string>;
  rating: RatingFilters;
  /** null: not in the URL (the saved preference applies) */
  sort: SearchSort | null;
};

const FILTER_KEYS = ['genre', 'lang', 'ext', 'sort', 'minMy', 'minLib', 'minExt', 'minExtVotes', 'unrated', 'kids'];

function list(v: string | null): string[] {
  return (v ?? '').split(',').map((s) => s.trim()).filter(Boolean);
}
function num(v: string | null, min: number, max: number): number {
  const n = Number(v);
  return Number.isFinite(n) && n >= min && n <= max ? n : 0;
}

export function parseSearchFilters(search: string): SearchFilters {
  const p = new URLSearchParams(search);
  const rating = emptyRatingFilters();
  rating.minMy = Math.round(num(p.get('minMy'), 0, 5));
  rating.minLib = Math.round(num(p.get('minLib'), 0, 5));
  rating.minExt = num(p.get('minExt'), 0, 5);
  rating.minExtVotes = Math.round(num(p.get('minExtVotes'), 0, 1e9));
  rating.unratedByMe = p.get('unrated') === '1';
  const kids = p.get('kids');
  rating.kidsMaxAge = kids !== null && kids !== '' && Number.isFinite(Number(kids)) ? Number(kids) : null;
  const sort = p.get('sort') as SearchSort | null;
  return {
    genre: new Set(list(p.get('genre')).map(Number).filter((n) => Number.isInteger(n) && n >= 0)),
    lang: new Set(list(p.get('lang'))),
    ext: new Set(list(p.get('ext')).map((s) => s.toLowerCase())),
    rating,
    sort: sort && SORTS.includes(sort) ? sort : null,
  };
}

/** `search` with the filter parameters replaced by `f` (other parameters kept, in order). */
export function withSearchFilters(search: string, f: SearchFilters): string {
  const p = new URLSearchParams(search);
  for (const k of FILTER_KEYS) p.delete(k);
  if (f.genre.size) p.set('genre', [...f.genre].join(','));
  if (f.lang.size) p.set('lang', [...f.lang].join(','));
  if (f.ext.size) p.set('ext', [...f.ext].join(','));
  const r = f.rating;
  if (r.minMy) p.set('minMy', String(r.minMy));
  if (r.minLib) p.set('minLib', String(r.minLib));
  if (r.minExt) p.set('minExt', String(r.minExt));
  if (r.minExtVotes) p.set('minExtVotes', String(r.minExtVotes));
  if (r.unratedByMe) p.set('unrated', '1');
  if (r.kidsMaxAge !== null) p.set('kids', String(r.kidsMaxAge));
  if (f.sort) p.set('sort', f.sort);
  const s = p.toString().replace(/%2C/g, ',');
  return s ? `?${s}` : '';
}
