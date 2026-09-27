import { test, expect } from '@playwright/test';

// The Libraries card counts like the rest of the app (live books, works, authors / series with a
// live book = the name lists), says what happens to deleted books, and the "Don't store deleted
// books" option explains itself and re-imports when changed.
test('library card counts and the deleted-books option', async ({ page, request }) => {
  const libs = (await (await request.get('/api/v1/libraries')).json()) as {
    id: number; name: string; bookCount: number; workCount: number; deletedCount: number;
    authorCount: number; seriesCount: number; skipDeleted: boolean;
  }[];
  const lib = libs.find((l) => l.name === 'Home Collection')!;
  const authors = (await (await request.get(`/api/v1/libraries/${lib.id}/authors`)).json()).rows as unknown[];
  const series = (await (await request.get(`/api/v1/libraries/${lib.id}/series`)).json()).rows as unknown[];
  expect(lib.authorCount).toBe(authors.length);
  expect(lib.seriesCount).toBe(series.length);
  expect(lib.workCount).toBeGreaterThan(0);
  expect(lib.workCount).toBeLessThanOrEqual(lib.bookCount);
  expect(lib.deletedCount).toBeGreaterThan(0);

  await page.goto('/libraries');
  const card = page.locator('.card', { has: page.getByRole('heading', { name: lib.name }) });
  const fmt = (n: number) => n.toLocaleString('en');
  await expect(card.getByTestId('lib-books')).toContainText(fmt(lib.bookCount));
  await expect(card.getByTestId('lib-works')).toContainText(`${fmt(lib.workCount)} works`);
  await expect(card.getByTestId('lib-authors')).toContainText(fmt(lib.authorCount));
  await expect(card.getByTestId('lib-deleted')).toHaveText(`${fmt(lib.deletedCount)} deleted in the library: hidden, opened only by link`);

  await card.getByRole('button', { name: 'Edit' }).click();
  const dialog = page.getByRole('dialog');
  const box = dialog.getByRole('checkbox', { name: "Don't store deleted books" });
  await expect(box).not.toBeChecked();
  await expect(dialog.getByText('always hidden from lists and search')).toBeVisible();
  await box.check();
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText(`Import options changed — re-importing «${lib.name}».`)).toBeVisible();
  await expect(card.getByTestId('lib-deleted')).toHaveText(`${fmt(lib.deletedCount)} deleted in the library: not stored`, { timeout: 10000 });

  // back as it was (re-imports again)
  await request.patch(`/api/v1/libraries/${lib.id}`, { data: { skipDeleted: false } });
});
