import { test, expect } from '@playwright/test';

test('browse an author, select books, open send dialog', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Authors' })).toBeVisible({ timeout: 15000 });

  // The seeded Strugatsky author is filterable by name.
  await page.getByLabel('Filter authors').fill('Стругацкий Аркадий Натанович');
  await page.getByRole('button', { name: /Стругацкий Аркадий Натанович/ }).first().click();

  await expect(page.getByText('Трудно быть богом')).toBeVisible();

  // Select two books via their row checkboxes.
  await page.getByRole('checkbox', { name: 'Select Трудно быть богом' }).check();
  await page.getByRole('checkbox', { name: 'Select Пикник на обочине' }).check();

  await expect(page.getByText('2 selected')).toBeVisible();
  await page.getByRole('button', { name: 'Send to…' }).click();

  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(page.getByText('Трудно быть богом, Пикник на обочине')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Kindle Send by email EPUB' })).toBeVisible();
});
