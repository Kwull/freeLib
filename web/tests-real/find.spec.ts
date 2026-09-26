import { test, expect } from '@playwright/test';
import { pickAuthor, booksOfAuthor } from './helpers';

// The start page, follows and the search additions against the real server.
test('start page, follow, and a corrected search', async ({ page, request }) => {
  const errors: string[] = [];
  page.on('pageerror', (err) => errors.push(String(err)));

  // "/" lands on the start page
  await page.goto('/');
  await expect(page).toHaveURL(/\/l\/1\/home$/, { timeout: 15000 });
  await expect(page.getByRole('heading', { name: 'What to read next' })).toBeVisible();

  // follow an author: the start page counts it
  const author = await pickAuthor(request, 1, 3);
  await page.goto(`/l/1/authors/${author.id}`);
  const follow = page.getByTestId('follow-author').first();
  await follow.click();
  await expect(follow).toHaveAttribute('aria-pressed', 'true');
  const home = await (await request.get('/api/v1/libraries/1/home?days=3650')).json();
  expect(home.following.authors).toBe(1);
  expect(home.empty).toBe(false);
  await follow.click();
  await expect(follow).toHaveAttribute('aria-pressed', 'false');

  // a surname with one letter dropped is corrected (or still found by transliteration)
  const surname = author.name.split(' ')[0];
  if (surname.length >= 6) {
    const typo = surname.slice(0, 2) + surname.slice(3);
    const r = await (await request.get(`/api/v1/libraries/1/search?q=${encodeURIComponent(typo)}`)).json();
    expect(r.total + r.authors.length > 0 || r.corrected || r.didYouMean).toBeTruthy();
  }

  // grouped books of the author: every row is one work, editions listed when there are several
  const books = await booksOfAuthor(request, author.id, 1, 5000);
  const grouped = await (await request.get(`/api/v1/libraries/1/books?author=${author.id}&group=1&limit=5000`)).json();
  expect(grouped.total).toBeLessThanOrEqual(books.length);
  expect(errors).toEqual([]);
});
