import { test, expect } from '@playwright/test';
import { pickAuthor } from './helpers';

test('search finds an author and renders the authors section', async ({ page, request }) => {
  const author = await pickAuthor(request, 1, 1);
  // A word from the author's name is enough to hit them via the sort-key prefix match.
  const word = author.name.split(' ').find((w) => w.length >= 3) ?? author.name;

  await page.goto(`/l/1/search?q=${encodeURIComponent(word)}`);
  await expect(page.getByText(/results for/)).toBeVisible({ timeout: 15000 });

  await expect(page.getByRole('heading', { name: 'AUTHORS' })).toBeVisible();
  await expect(page.locator('.series-card', { hasText: author.name })).toBeVisible();
});

test('search with a facet filter', async ({ page, request }) => {
  const author = await pickAuthor(request, 1, 1);
  const word = author.name.split(' ').find((w) => w.length >= 3) ?? author.name;

  await page.goto(`/l/1/search?q=${encodeURIComponent(word)}`);
  await expect(page.getByText(/results for/)).toBeVisible({ timeout: 15000 });

  const genreCheckboxes = page.locator('.facet .fl input[type="checkbox"]');
  const count = await genreCheckboxes.count();
  if (count > 0) {
    await genreCheckboxes.first().check();
    await expect(page.locator('.summary .chip').first()).toBeVisible();
  }
});
