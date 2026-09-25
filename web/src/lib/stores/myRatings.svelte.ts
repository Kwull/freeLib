// Ratings the user changed in this session, so every open list shows (and sorts by) the new
// value without reloading: `${lib}:${bookId}` → rating.
export const myRatings = $state<{ changed: Record<string, number> }>({ changed: {} });

export function noteRating(lib: number, id: number, rating: number) {
  myRatings.changed = { ...myRatings.changed, [`${lib}:${id}`]: rating };
}
