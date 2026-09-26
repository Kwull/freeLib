import { test, expect } from '@playwright/test';
import { pickAuthor, booksOfAuthor } from './helpers';

// Regression test for the missing-cover 404s: book grids/lists used to request
// `/cover?size=thumb` for every book and log a 404 for each one without a real cover. The
// server now answers with a generated placeholder SVG instead (see server/crates/server/src/
// placeholder.rs), so browsing should never print a console error.
test('no console errors browsing authors, a book list and search', async ({ page, request }) => {
  const errors: string[] = [];
  page.on('console', (msg) => { if (msg.type() === 'error') errors.push(msg.text()); });
  page.on('pageerror', (err) => errors.push(String(err)));

  const author = await pickAuthor(request, 1, 3);
  const books = await booksOfAuthor(request, author.id, 1, 10);

  await page.goto('/l/1/authors');
  await expect(page.getByRole('heading', { name: 'Authors' })).toBeVisible({ timeout: 15000 });

  await page.getByLabel('Filter authors').fill(author.name);
  await page.getByRole('button', { name: new RegExp(author.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }).first().click();
  if (books[0]) await expect(page.getByText(books[0].title)).toBeVisible();

  // Cover (grid) view renders CoverThumb tiles for every book too.
  const gridToggle = page.getByRole('button', { name: 'Cover view' });
  if (await gridToggle.count()) await gridToggle.click();
  await page.waitForTimeout(500);

  await page.goto(`/l/1/search?q=${encodeURIComponent(author.name.split(' ')[0])}`);
  await expect(page.getByText(/results for/)).toBeVisible({ timeout: 15000 });
  await page.waitForTimeout(500);

  expect(errors, `console errors: ${errors.join('\n')}`).toEqual([]);
});
