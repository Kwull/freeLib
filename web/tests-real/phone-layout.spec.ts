import { test, expect, devices } from '@playwright/test';
import { pickAuthor, booksOfAuthor } from './helpers';

// Chromium only is installed in this environment, so take the iPhone 13's viewport/UA but keep
// rendering on chromium rather than webkit.
test.use({ ...devices['iPhone 13'], defaultBrowserType: undefined, browserName: undefined });

test('phone layout navigates list -> books -> detail', async ({ page, request }) => {
  const author = await pickAuthor(request, 1, 1);
  const books = await booksOfAuthor(request, author.id, 1, 1);
  test.skip(books.length < 1, 'picked author has no books');

  await page.goto('/l/1/authors');
  await expect(page.locator('nav[aria-label="Main"]').last()).toBeVisible({ timeout: 15000 });

  await expect(page.getByLabel('Filter authors')).toBeVisible();

  await page.getByLabel('Filter authors').fill(author.name);
  await page.getByRole('button', { name: new RegExp(author.name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }).first().click();

  const bookRow = page.locator('.m-row', { hasText: books[0].title });
  await expect(bookRow).toBeVisible();

  await bookRow.click();
  await expect(page).toHaveURL(/\/book\//);
});
