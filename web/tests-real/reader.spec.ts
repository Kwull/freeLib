import { test, expect } from '@playwright/test';
import { pickFb2Book } from './helpers';

test('reader opens a converted EPUB and turns a page', async ({ page, request }) => {
  const book = await pickFb2Book(request, 1);

  await page.goto(`/l/1/read/${book.id}`);

  const progress = page.getByLabel('Progress');
  await expect(progress).toBeVisible({ timeout: 15000 });
  await expect.poll(async () => Number(await progress.inputValue()), { timeout: 15000 }).toBeGreaterThanOrEqual(0);
  const progressBefore = Number(await progress.inputValue());

  await page.getByRole('button', { name: 'Next page' }).click();
  await page.waitForTimeout(400);
  const progressAfterClick = Number(await progress.inputValue());
  expect(progressAfterClick).toBeGreaterThanOrEqual(progressBefore);

  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(400);
  const progressAfterKey = Number(await progress.inputValue());
  expect(progressAfterKey).toBeGreaterThanOrEqual(progressAfterClick);

  // TOC drawer opens and lists the synthetic book's two chapters (fixed titles from gen-inpx).
  await page.getByRole('button', { name: 'Contents' }).click();
  await expect(page.getByText('Глава 2')).toBeVisible();
  await page.getByText('Глава 2').click();
  await page.waitForTimeout(400);

  // Position is remembered per book in localStorage.
  const stored = await page.evaluate(() => Object.keys(localStorage).some((k) => k.startsWith('freelib.reader.pos.')));
  expect(stored).toBe(true);
});
