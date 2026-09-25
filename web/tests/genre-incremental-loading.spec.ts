import { test, expect } from '@playwright/test';

// A genre can have tens of thousands of books in the real app, so BooksPane
// pages them in as the virtual list scrolls near the end instead of loading
// everything up front. This drives that scroll and checks a follow-up,
// cursor-bearing request actually fires.
test('genre books load incrementally as the list scrolls', async ({ page }) => {
  const bookRequests: string[] = [];
  page.on('request', (req) => {
    const url = new URL(req.url());
    if (url.pathname === '/api/v1/libraries/1/books' && url.searchParams.get('genre')) {
      bookRequests.push(url.search);
    }
  });

  await page.goto('/l/1/genres/1');
  await expect(page.locator('.scroll .brow.head, .brow.head')).toBeVisible({ timeout: 15000 });

  // Initial page loads without a cursor.
  await expect.poll(() => bookRequests.length, { timeout: 15000 }).toBeGreaterThan(0);
  expect(bookRequests[0]).not.toContain('cursor=');
  const initialCount = bookRequests.length;

  // Scroll the virtualized table body to (near) the bottom several times so
  // VirtualList's onRangeChange reports we're near the end and BooksPane
  // fetches the next cursor page.
  const scroller = page.locator('.table-wrap .scroll .vlist').first();
  for (let i = 0; i < 8; i++) {
    await scroller.evaluate((el) => { el.scrollTop = el.scrollHeight; });
    await page.waitForTimeout(250);
  }

  await expect.poll(() => bookRequests.length, { timeout: 15000 }).toBeGreaterThan(initialCount);
  expect(bookRequests.some((q) => q.includes('cursor='))).toBe(true);
});
