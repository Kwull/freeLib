import { test, expect, type Page } from '@playwright/test';

async function openDoyle(page: Page) {
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
  await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
  await expect(page.getByText('The Hound of the Baskervilles')).toBeVisible();
}

async function titles(page: Page): Promise<string[]> {
  return page.locator('.scroll .brow .title-btn').allTextContents();
}

test('rating columns, client-side sort and filters on an author', async ({ page }) => {
  await openDoyle(page);
  // optional columns
  await page.getByRole('button', { name: 'Columns' }).click();
  for (const c of ['My rating', 'Library rating', 'Open Library']) {
    await page.getByRole('menu').getByLabel(c).check();
  }
  await page.getByRole('button', { name: 'Columns' }).click();
  const head = page.locator('.brow.head');
  await expect(head).toContainText('My rating');
  await expect(head).toContainText('Library');
  await expect(head).toContainText('Open Library');
  const hound = page.locator('.brow', { hasText: 'The Hound of the Baskervilles' });
  await expect(hound).toContainText('4.4');
  await expect(hound).toContainText('(2.1k)');
  await expect(hound.getByTestId('kids-badge')).toHaveText('12+');

  // sort by library rating: best first, the list order among equals (series, number)
  await page.getByTestId('books-sort').selectOption('libRating');
  const t = await titles(page);
  expect(t.slice(0, 4)).toEqual(['The Lost World', 'A Study in Scarlet', 'The Sign of the Four', 'The Hound of the Baskervilles']);
  // sort by Open Library rating
  await page.getByTestId('books-sort').selectOption('extRating');
  expect((await titles(page))[0]).toBe('The Hound of the Baskervilles');

  // filters: library rating ≥ 5, then Open Library ≥ 4 with ≥ 100 votes
  await page.getByRole('button', { name: /Filter/ }).click();
  await page.getByTestId('filter-min-lib').selectOption('5');
  await expect(page.getByText('4 of 15')).toBeVisible();
  await page.getByTestId('filter-min-lib').selectOption('0');
  await page.getByTestId('filter-min-ext').selectOption('4');
  await page.getByTestId('filter-min-votes').selectOption('100');
  await expect(page.getByText('7 of 15')).toBeVisible();
  await page.getByTestId('filter-min-votes').selectOption('0');
  await page.getByTestId('filter-min-ext').selectOption('0');

  // rate a book, then "not rated by me" hides it and "my rating" sort puts it first
  await page.getByRole('button', { name: /Filter/ }).click();
  await page.locator('.brow', { hasText: 'Micah Clarke' }).click();
  await page.getByTestId('my-rating').getByRole('button', { name: 'Rate 4' }).click();
  await page.getByTestId('books-sort').selectOption('myRating');
  await expect.poll(async () => (await titles(page))[0]).toBe('Micah Clarke');
  await page.getByRole('button', { name: /Filter/ }).click();
  await page.getByTestId('filter-unrated').check();
  await expect(page.getByText('14 of 15')).toBeVisible();
});

test('details pane shows all three ratings and the age estimate', async ({ page }) => {
  await openDoyle(page);
  await page.getByText('The Hound of the Baskervilles').first().click();
  const d = page.getByRole('complementary', { name: 'Details' });
  await expect(d.getByTestId('lib-rating')).toBeVisible();
  await expect(d.getByTestId('ext-rating')).toContainText('4.4');
  await expect(d.getByTestId('ext-rating')).toContainText('2104 votes');
  await expect(d.getByTestId('ext-rating').getByRole('link', { name: 'openlibrary.org' })).toHaveAttribute('href', /openlibrary\.org\/works\/OL/);
  await expect(d.getByTestId('kids-age')).toContainText('12+');
  await expect(d.getByTestId('my-rating').getByRole('button', { name: 'Rate 5' })).toBeVisible();
});

test('genre lists are filtered and sorted by rating on the server', async ({ page }) => {
  await page.goto('/l/1/genres/2');
  await expect(page.locator('.scroll .brow').first()).toBeVisible({ timeout: 15000 });
  const req = page.waitForRequest((r) => r.url().includes('/books?') && r.url().includes('sort=ext'));
  await page.getByTestId('books-sort').selectOption('extRating');
  await (await req).response();
  await page.getByRole('button', { name: 'Columns' }).click();
  await page.getByRole('menu').getByLabel('Open Library').check();
  await page.getByRole('button', { name: 'Columns' }).click();
  // the sorted page and the new column render after the response: wait for the values
  await expect.poll(() => page.locator('.scroll .brow .ext .avg').count(), { timeout: 10000 }).toBeGreaterThan(3);
  const avgs = (await page.locator('.scroll .brow .ext .avg').allTextContents()).map(Number);
  expect(avgs.length).toBeGreaterThan(3);
  for (let i = 1; i < avgs.length; i++) expect(avgs[i - 1]).toBeGreaterThanOrEqual(avgs[i]);

  // kids filter: sent to the server, every shown badge within the age
  await page.goto('/l/1/genres/18');
  await expect(page.locator('.scroll .brow').first()).toBeVisible({ timeout: 15000 });
  await page.getByRole('button', { name: /Filter/ }).click();
  const kreq = page.waitForRequest((r) => r.url().includes('kidsMaxAge=6'));
  await page.getByTestId('filter-kids').selectOption('6');
  await kreq;
  await expect.poll(async () => {
    const b = await page.locator('.scroll .brow [data-testid=kids-badge]').allTextContents();
    return b.length > 0 && b.every((x) => x === '0+' || x === '6+');
  }).toBe(true);
});

test('search sorts by rating', async ({ page }) => {
  await page.goto('/l/1/search?q=the');
  await expect(page.getByTestId('search-sort')).toBeVisible({ timeout: 15000 });
  const req = page.waitForRequest((r) => r.url().includes('/search?') && r.url().includes('sort=lib'));
  await page.getByTestId('search-sort').selectOption('libRating');
  await req;
  const minReq = page.waitForRequest((r) => r.url().includes('/search?') && r.url().includes('minLib=4'));
  await page.getByTestId('filter-min-lib').selectOption('4');
  await minReq;
});

test('wide tables: checkbox and Title stay while other columns scroll; a new sort starts at the left', async ({ page }) => {
  await page.goto('/l/1/genres/2');
  await expect(page.locator('.scroll .brow').first()).toBeVisible({ timeout: 15000 });
  await page.getByRole('button', { name: 'Columns' }).click();
  for (const c of ['My rating', 'Library rating', 'Open Library', 'Genre', 'Series', 'Size']) await page.getByRole('menu').getByLabel(c, { exact: true }).check();
  await page.getByRole('button', { name: 'Columns' }).click();
  await page.locator('.scroll .brow .title-btn').first().click(); // details pane opens: the table gets narrower
  const vlist = page.locator('.scroll .vlist');
  const title = page.locator('.scroll .brow .title-cell').first();
  const x0 = (await title.boundingBox())!.x;
  expect((await title.boundingBox())!.width).toBeGreaterThanOrEqual(219);
  await vlist.evaluate((el) => (el.scrollLeft = el.scrollWidth));
  await expect.poll(() => vlist.evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
  expect(Math.abs((await title.boundingBox())!.x - x0)).toBeLessThan(1);
  expect(await page.locator('.head-wrap').evaluate((el) => el.scrollLeft)).toBeGreaterThan(0);
  await page.getByTestId('books-sort').selectOption('libRating');
  await expect.poll(() => vlist.evaluate((el) => el.scrollLeft)).toBe(0);
  await expect.poll(() => page.locator('.head-wrap').evaluate((el) => el.scrollLeft)).toBe(0);
});
