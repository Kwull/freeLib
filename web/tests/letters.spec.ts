import { test, expect } from '@playwright/test';

test.use({ viewport: { width: 1440, height: 900 } });

test('letter strip: one script at a time, diacritics folded, empty letters disabled, # last', async ({ page }) => {
  await page.goto('/l/1/authors');
  const scripts = page.getByRole('group', { name: 'Alphabet' });
  await expect(scripts).toBeVisible({ timeout: 15000 });
  // the mock library is mostly Latin: A–Z first and selected
  await expect(scripts.getByRole('button')).toHaveText(['A–Z', 'А–Я', '#']);
  await expect(scripts.getByRole('button', { name: 'A–Z' })).toHaveAttribute('aria-pressed', 'true');

  const strip = page.getByLabel('Jump to letter');
  await expect(strip.getByRole('button')).toHaveCount(26);
  await expect(strip).not.toContainText('Č');
  await expect(strip).not.toContainText('Ø');
  // letters with no names are there but disabled
  await expect(strip.getByRole('button', { name: /^E: 0$/ })).toBeDisabled();

  // Čapek sorts and indexes under C
  await page.getByLabel('Filter authors').fill('capek');
  await expect(page.getByRole('button', { name: /Čapek Karel/ })).toBeVisible();
  await page.getByLabel('Filter authors').fill('');

  await scripts.getByRole('button', { name: 'А–Я' }).click();
  await expect(strip.getByRole('button', { name: /^А: \d+$/ })).toBeEnabled();
  await expect(page.getByRole('button', { name: /Азимов Айзек/ })).toBeVisible();

  // digits and symbols come last, in the # group
  await scripts.getByRole('button', { name: '#' }).click();
  await expect(page.getByRole('button', { name: /1984 Group/ })).toBeVisible();
  const rows = await page.request.get('/api/v1/libraries/1/authors').then((r) => r.json());
  expect(rows.rows.at(-1)[1]).toBe('4PDA Team');
  expect(rows.letters.at(-1)[0]).toBe('#');
});

test('breadcrumb uses the index letter', async ({ page }) => {
  await page.goto('/');
  await page.getByLabel('Filter authors').fill('1984');
  await page.getByRole('button', { name: /1984 Group/ }).click();
  await expect(page.locator('.scope-header .crumb')).toHaveText('Authors / #');
  await page.getByLabel('Filter authors').fill('capek');
  await page.getByRole('button', { name: /Čapek Karel/ }).click();
  await expect(page.locator('.scope-header .crumb')).toHaveText('Authors / C');
});
