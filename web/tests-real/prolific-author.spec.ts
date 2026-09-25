import { test, expect } from '@playwright/test';
import { LIB } from './helpers';

// gen-inpx makes "Азимов Айзек" the most prolific author, with many series and hundreds of
// anthologies (so thousands of names share a book with him).
test.use({ viewport: { width: 1440, height: 900 } });

test('prolific author: compact header, summary, collapsed series, co-author list', async ({ page, request }) => {
  const rows = (await (await request.get(`/api/v1/libraries/${LIB}/authors`)).json()).rows as [number, string, number][];
  const top = rows.reduce((a, b) => (b[2] > a[2] ? b : a));
  const summary = await (await request.get(`/api/v1/libraries/${LIB}/authors/${top[0]}/summary`)).json();
  expect(summary.count).toBe(top[2]);
  expect(summary.coauthorCount).toBeGreaterThan(20);

  await page.goto(`/l/${LIB}/authors/${top[0]}`);
  const header = page.locator('.scope-header');
  await expect(header.getByRole('heading', { level: 1 })).toHaveText(top[1]);
  await expect(header.locator('.coauthors a')).toHaveCount(Math.min(3, summary.coauthors.length));
  expect((await header.boundingBox())!.height).toBeLessThan(170);
  await header.getByRole('button', { name: /and \d+ more/ }).click();
  const pop = page.getByRole('dialog', { name: 'Co-authors' });
  await expect(pop.locator('.r').first()).toBeVisible();
  await page.keyboard.press('Escape');

  // more than 8 series: groups start collapsed
  if (summary.series.length > 8) {
    await expect(page.locator('.group-head').first()).toBeVisible();
    await expect(page.locator('.scroll .brow')).toHaveCount(0);
  }
  const details = page.getByRole('complementary', { name: 'Details' });
  await expect(details.getByText('About the author')).toBeVisible();

  // the filter box searches the whole bibliography
  const firstTitle = (await (await request.get(`/api/v1/libraries/${LIB}/books?author=${top[0]}&limit=1`)).json()).books[0].title as string;
  await page.getByPlaceholder('Find in these books').fill(firstTitle);
  await expect(page.locator('.scroll .brow', { hasText: firstTitle }).first()).toBeVisible();
});

test('pane widths persist through /me/prefs', async ({ page, request }) => {
  await page.goto(`/l/${LIB}/authors`);
  const list = page.locator('section.browser');
  await expect(list).toBeVisible({ timeout: 15000 });
  const w0 = (await list.boundingBox())!.width;
  const handle = page.getByRole('separator', { name: 'Resize the list' });
  await handle.focus();
  await page.keyboard.press('Shift+ArrowRight');
  await expect.poll(async () => (await list.boundingBox())!.width).toBeGreaterThan(w0 + 50);
  await expect.poll(async () => ((await (await request.get('/api/v1/me/prefs')).json()).panes ?? {}).list, { timeout: 5000 }).toBe(Math.round(w0) + 64);
  // reset for the other specs
  await request.put('/api/v1/me/prefs', { data: {} });
});
