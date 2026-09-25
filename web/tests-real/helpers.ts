// Picks real data out of the synthetic library at test time, since real-mode specs cannot rely
// on mock-only fixtures like the seeded Strugatsky author.
import type { APIRequestContext } from '@playwright/test';

export const LIB = 1;

export type AuthorRow = { id: number; name: string; count: number };

export async function pickAuthor(request: APIRequestContext, lib = LIB, minBooks = 2): Promise<AuthorRow> {
  const res = await request.get(`/api/v1/libraries/${lib}/authors`);
  const body = (await res.json()) as { rows: [number, string, number][] };
  if (body.rows.length === 0) throw new Error('no authors in the synthetic library');
  const row = body.rows.find((r) => r[2] >= minBooks) ?? body.rows[0];
  return { id: row[0], name: row[1], count: row[2] };
}

export async function booksOfAuthor(request: APIRequestContext, authorId: number, lib = LIB, limit = 5) {
  const res = await request.get(`/api/v1/libraries/${lib}/books?author=${authorId}&limit=${limit}`);
  const body = (await res.json()) as { books: { id: number; title: string; ext: string }[] };
  return body.books;
}

/// Any FB2 book (fb2conv can always produce a real reader-openable EPUB from it; the synthetic
/// generator's non-FB2 "files" are placeholder text, not real documents).
export async function pickFb2Book(request: APIRequestContext, lib = LIB) {
  const res = await request.get(`/api/v1/libraries/${lib}/books?since=1900-01-01&ext=fb2&limit=1`);
  const body = (await res.json()) as { books: { id: number; title: string }[] };
  if (body.books.length === 0) throw new Error('no FB2 books in the synthetic library');
  return body.books[0];
}

/// The genre with the most books, for the incremental-loading test (needs > one page, i.e. more
/// than 100 books, to actually exercise cursor pagination).
export async function pickBiggestGenre(request: APIRequestContext, lib = LIB) {
  const res = await request.get(`/api/v1/libraries/${lib}/genres`);
  const body = (await res.json()) as { id: number; count: number }[];
  return body.reduce((a, b) => (b.count > a.count ? b : a));
}
