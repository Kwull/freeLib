import { test, expect } from '@playwright/test';

test('browse an author, select books, open send dialog', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Authors' })).toBeVisible({ timeout: 15000 });

  // The seeded Strugatsky author is filterable by name.
  await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
  await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();

  await expect(page.getByText('The Hound of the Baskervilles')).toBeVisible();

  // Select two books via their row checkboxes.
  await page.getByRole('checkbox', { name: 'Select The Hound of the Baskervilles' }).check();
  await page.getByRole('checkbox', { name: 'Select The White Company' }).check();

  await expect(page.getByText('2 selected')).toBeVisible();
  await page.getByRole('button', { name: 'Send to…' }).click();

  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText('«The Hound of the Baskervilles», «The White Company»')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Kindle Send by email EPUB' })).toBeVisible();
});
