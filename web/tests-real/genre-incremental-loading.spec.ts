import { test, expect } from '@playwright/test';
import { pickBiggestGenre } from './helpers';

// Mirrors tests/genre-incremental-loading.spec.ts against real data: picks whichever genre has
// the most books in the synthetic library (start-server.sh generates enough books that some
// genre passes the 100-book first-page size).
test('genre books load incrementally as the list scrolls', async ({ page, request }) => {
  const genre = await pickBiggestGenre(request, 1);
  test.skip(genre.count <= 100, `no genre with > 100 books (biggest has ${genre.count})`);

  const bookRequests: string[] = [];
  page.on('request', (req) => {
    const url = new URL(req.url());
    if (url.pathname === '/api/v1/libraries/1/books' && url.searchParams.get('genre')) {
      bookRequests.push(url.search);
    }
  });

  await page.goto(`/l/1/genres/${genre.id}`);
  await expect(page.locator('.scroll .brow.head, .brow.head')).toBeVisible({ timeout: 15000 });

  await expect.poll(() => bookRequests.length, { timeout: 15000 }).toBeGreaterThan(0);
  expect(bookRequests[0]).not.toContain('cursor=');
  const initialCount = bookRequests.length;

  const scroller = page.locator('.table-wrap .scroll .vlist').first();
  for (let i = 0; i < 8; i++) {
    await scroller.evaluate((el) => { el.scrollTop = el.scrollHeight; });
    await page.waitForTimeout(250);
  }

  await expect.poll(() => bookRequests.length, { timeout: 15000 }).toBeGreaterThan(initialCount);
  expect(bookRequests.some((q) => q.includes('cursor='))).toBe(true);
});
