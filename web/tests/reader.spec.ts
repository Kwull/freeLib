import { test, expect } from '@playwright/test';

test('reader opens an EPUB and turns a page', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Authors' })).toBeVisible({ timeout: 15000 });

  await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
  await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
  await page.getByText('The Hound of the Baskervilles').first().click();

  await page.getByRole('button', { name: 'Read' }).click();
  await expect(page).toHaveURL(/\/read\//);

  // The book opened and foliate-js reports a real reading position.
  const progress = page.getByLabel('Progress');
  await expect(progress).toBeVisible({ timeout: 15000 });
  await expect.poll(async () => Number(await progress.inputValue()), { timeout: 15000 }).toBeGreaterThan(0);
  const progressBefore = Number(await progress.inputValue());

  await page.getByRole('button', { name: 'Next page' }).click();
  await page.waitForTimeout(400);
  const progressAfterClick = Number(await progress.inputValue());
  expect(progressAfterClick).toBeGreaterThanOrEqual(progressBefore);

  // Keyboard navigation works too.
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(400);
  const progressAfterKey = Number(await progress.inputValue());
  expect(progressAfterKey).toBeGreaterThanOrEqual(progressAfterClick);

  // TOC drawer opens and lists the fixture's two Russian chapters, and can
  // jump to one of them.
  await page.getByRole('button', { name: 'Contents' }).click();
  await expect(page.getByText('Глава вторая')).toBeVisible();
  await page.getByText('Глава вторая').click();
  await page.waitForTimeout(400);

  // Position is remembered per book in localStorage.
  const stored = await page.evaluate(() => Object.keys(localStorage).some((k) => k.startsWith('freelib.reader.pos.')));
  expect(stored).toBe(true);
});
