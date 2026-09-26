import { test, expect, devices } from '@playwright/test';

// ISBN (from the FB2 <publish-info>, checksum-verified, both forms), publisher and year in the
// book details: the desktop pane and the phone book page.

test('desktop: ISBN with a copy action, publisher and year', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.setViewportSize({ width: 2000, height: 836 });
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill('Азимов Айзек');
  await page.getByRole('button', { name: /Азимов Айзек/ }).first().click();
  await page.locator('.scroll .brow', { hasText: /Академия и Земля|Основание и Земля/ }).first().click();
  const details = page.getByRole('complementary', { name: 'Details' });
  const isbn = details.getByTestId('isbn');
  await expect(isbn).toContainText(/978-5-\d{3}-\d{5}-\d/);
  await expect(isbn).toContainText(/ISBN-10 5\d{8}[\dX]/);
  await expect(details.getByTestId('publisher')).toContainText(/, 20\d\d$/);
  const copy = isbn.getByRole('button', { name: /Copy ISBN 978\d{10}/ });
  await copy.click();
  await expect(page.getByText('Copied')).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toMatch(/^978\d{10}$/);
  await page.screenshot({ path: 'test-results/screenshots/fixes-isbn.png' });
});

test.describe('phone', () => {
  const { defaultBrowserType: _b, ...iphone } = devices['iPhone 13'];
  test.use({ ...iphone });

  test('phone book page shows the ISBN', async ({ page, request }) => {
    const books = (await (await request.get('/api/v1/libraries/1/books?since=1900-01-01&ext=fb2&limit=40')).json()).books as { id: number }[];
    let id = 0;
    for (const b of books) {
      const d = await (await request.get(`/api/v1/libraries/1/books/${b.id}`)).json();
      if (d.isbn?.length) { id = b.id; break; }
    }
    expect(id).toBeGreaterThan(0);
    await page.goto(`/l/1/book/${id}`);
    await expect(page.getByTestId('isbn')).toContainText(/978-5-/);
    await expect(page.getByTestId('isbn').getByRole('button', { name: /Copy ISBN/ })).toBeVisible();
  });
});
