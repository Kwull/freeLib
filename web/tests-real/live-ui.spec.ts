import { test, expect } from '@playwright/test';
import { pickFb2Book } from './helpers';

// Against the real server: a bad reader link shows a not-found card (the API answers 404),
// leaving the reader leaves no errors behind, and a folder device without a folder is refused
// with a clear message.

test('reader: a bad book id shows "Book not found", a real book cleans up when left', async ({ page, request }) => {
  const errors: string[] = [];
  page.on('console', (msg) => { if (msg.type() === 'error' && !/404/.test(msg.text())) errors.push(msg.text()); });
  page.on('pageerror', (err) => errors.push(String(err)));

  await page.goto('/l/1/read/999999999');
  await expect(page.getByTestId('reader-not-found')).toContainText('Book not found', { timeout: 15000 });

  const book = await pickFb2Book(request);
  await page.goto(`/l/1/read/${book.id}`);
  await expect(page.locator('.reader')).toHaveAttribute('data-ready', 'true', { timeout: 30000 });
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(300);
  await page.getByRole('button', { name: 'Back' }).first().click();
  await expect(page).not.toHaveURL(/\/read\//);
  await page.setViewportSize({ width: 900, height: 700 });
  await page.waitForTimeout(500);
  expect(errors, errors.join('\n')).toEqual([]);
});

test('a folder device without a folder is refused with a clear error', async ({ request }) => {
  const devices = (await (await request.get('/api/v1/devices')).json()) as { id: number; kind: string; target: string | null }[];
  const folder = devices.find((d) => d.kind === 'folder' && !(d.target ?? '').trim());
  test.skip(!folder, 'no folder device without a folder');
  const book = await pickFb2Book(request);
  const r = await request.post('/api/v1/send', { data: { library: 1, books: [book.id], device: folder!.id } });
  expect(r.status()).toBe(400);
  expect((await r.json()).message).toContain('no folder set');
});
