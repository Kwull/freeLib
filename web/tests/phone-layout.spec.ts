import { test, expect, devices } from '@playwright/test';

// Chromium only is installed in this environment, so take the iPhone 13's
// viewport/UA but keep rendering on chromium rather than webkit.
test.use({ ...devices['iPhone 13'], defaultBrowserType: undefined, browserName: undefined });

test('phone layout navigates list -> books -> detail', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('nav[aria-label="Main"]').last()).toBeVisible({ timeout: 15000 });

  // List pane visible first, books pane hidden.
  await expect(page.getByLabel('Filter authors')).toBeVisible();

  await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
  await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();

  const bookRow = page.locator('.m-row', { hasText: 'The Hound of the Baskervilles' });
  await expect(bookRow).toBeVisible();

  await bookRow.click();
  await expect(page).toHaveURL(/\/book\//);
});
