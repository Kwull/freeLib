// Rating filters and sorts shared by the book lists and search: the user's own rating, the
// library (INPX) rating, the Open Library rating and the (heuristic) kids age estimate.
import type { Book, RatingParams } from '../api/types';

export type RatingFilters = {
  /** minimum own rating 1..5, 0 = any */
  minMy: number;
  /** minimum library rating 1..5, 0 = any */
  minLib: number;
  /** minimum Open Library average (0..5), 0 = any */
  minExt: number;
  /** minimum Open Library vote count, 0 = any */
  minExtVotes: number;
  unratedByMe: boolean;
  /** only books whose age estimate is known and at most this; null = any */
  kidsMaxAge: number | null;
};

export type RatingSortKey = 'myRating' | 'libRating' | 'extRating';

export const RATING_SORTS: RatingSortKey[] = ['myRating', 'libRating', 'extRating'];
export const KIDS_AGES = [0, 6, 12, 16];

export function emptyRatingFilters(): RatingFilters {
  return { minMy: 0, minLib: 0, minExt: 0, minExtVotes: 0, unratedByMe: false, kidsMaxAge: null };
}

export function ratingFilterCount(f: RatingFilters): number {
  return (f.minMy ? 1 : 0) + (f.minLib ? 1 : 0) + (f.minExt ? 1 : 0) + (f.minExtVotes ? 1 : 0)
    + (f.unratedByMe ? 1 : 0) + (f.kidsMaxAge !== null ? 1 : 0);
}

/** Client-side filter (lists loaded in full: author, series). Same rules as the server. */
export function matchesRatingFilters(b: Book, f: RatingFilters): boolean {
  if (f.minMy && b.rating < f.minMy) return false;
  if (f.unratedByMe && b.rating > 0) return false;
  if (f.minLib && (b.libRating ?? 0) < f.minLib) return false;
  if (f.minExt || f.minExtVotes) {
    const e = b.extRating;
    if (!e || e.avg < f.minExt - 1e-9 || e.votes < f.minExtVotes) return false;
  }
  if (f.kidsMaxAge !== null && (b.kidsAge === null || b.kidsAge === undefined || b.kidsAge > f.kidsMaxAge)) return false;
  return true;
}

/** Sort key (higher = better, 0 = unrated). */
export function ratingKey(b: Book, key: RatingSortKey): number {
  if (key === 'myRating') return b.rating;
  if (key === 'libRating') return b.libRating ?? 0;
  const e = b.extRating;
  return e && e.votes > 0 ? e.avg * 1_000_000 + Math.min(e.votes, 999_999) : 0;
}

/** Stable sort by a rating, best first, unrated last (equal ratings keep the list order). */
export function sortByRating(list: Book[], key: RatingSortKey): Book[] {
  return list
    .map((b, i) => ({ b, i, k: ratingKey(b, key) }))
    .sort((x, y) => y.k - x.k || x.i - y.i)
    .map((x) => x.b);
}

/** Server query parameters for the filters and an optional rating sort. */
export function ratingParams(f: RatingFilters, sort?: RatingSortKey | null): RatingParams {
  const p: RatingParams = {};
  if (sort === 'myRating') p.sort = 'my';
  else if (sort === 'libRating') p.sort = 'lib';
  else if (sort === 'extRating') p.sort = 'ext';
  if (f.minMy) p.minMy = f.minMy;
  if (f.minLib) p.minLib = f.minLib;
  if (f.minExt) p.minExt = f.minExt;
  if (f.minExtVotes) p.minExtVotes = f.minExtVotes;
  if (f.unratedByMe) p.unratedByMe = true;
  if (f.kidsMaxAge !== null) p.kidsMaxAge = f.kidsMaxAge;
  return p;
}

export function kidsLabel(age: number | null | undefined): string {
  return age === null || age === undefined ? '' : `${age}+`;
}

/** "4.1" */
export function formatAvg(avg: number): string {
  return avg.toFixed(1);
}
