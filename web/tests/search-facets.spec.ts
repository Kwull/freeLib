import { test, expect } from '@playwright/test';

test('search with a facet filter', async ({ page }) => {
  await page.goto('/l/1/search?q=пикник');
  await expect(page.getByText(/results for/)).toBeVisible({ timeout: 15000 });
  await expect(page.getByRole('heading', { name: 'BOOKS' })).toBeVisible();

  const genreCheckboxes = page.locator('.facet .fl input[type="checkbox"]');
  const count = await genreCheckboxes.count();
  if (count > 0) {
    await genreCheckboxes.first().check();
    await expect(page.locator('.summary .chip').first()).toBeVisible();
  }
});
