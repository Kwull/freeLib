import { test, expect } from '@playwright/test';
import { pickFb2Book, LIB } from './helpers';

// The phone hand-off page of the real server: no session, strict CSP, "Open in Books" on iOS,
// an EPUB attachment.
test.use({
  viewport: { width: 390, height: 844 },
  userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
  isMobile: true,
  hasTouch: true,
});

test('phone hand-off link: page, cover and EPUB download', async ({ page, request }) => {
  const book = await pickFb2Book(request);
  const res = await request.post('/api/v1/handoff', {
    data: { library: LIB, book: book.id },
    headers: { 'Content-Type': 'application/json' },
  });
  expect(res.status()).toBe(200);
  const link = await res.json();
  expect(link.url).toMatch(/^\/h\/[A-Za-z0-9_-]{22}$/);

  const resp = await page.goto(link.url);
  expect(resp!.status()).toBe(200);
  expect(resp!.headers()['content-security-policy']).toContain("default-src 'none'");
  const button = page.getByRole('link', { name: 'Open in Books' });
  await expect(button).toBeVisible();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(book.title);
  await page.waitForFunction(() => {
    const img = document.querySelector('img');
    return !!img && img.complete;
  });
  await page.screenshot({ path: 'test-results/screenshots/delivery-phone-handoff.png' });

  const [download] = await Promise.all([page.waitForEvent('download'), button.click()]);
  expect(await download.failure()).toBeNull();
  // (headless Chromium without a UTF-8 locale names downloads with a non-ASCII filename*
  // "download", so the file name is checked on the header)
  const file = await request.get(`${link.url}/file`);
  expect(file.headers()['content-type']).toBe('application/epub+zip');
  expect(file.headers()['content-disposition']).toMatch(/^attachment; filename="[^"]+\.epub"; filename\*=UTF-8''.+\.epub$/);
  // three downloads per link (the button, the request above, this one), then it is gone
  expect((await request.get(`${link.url}/file`)).status()).toBe(200);
  expect((await request.get(`${link.url}/file`)).status()).toBe(410);
});
