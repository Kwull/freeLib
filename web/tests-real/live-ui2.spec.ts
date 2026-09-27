import { test, expect } from '@playwright/test';
import { pickAuthor, pickBiggestGenre } from './helpers';

// Against the real server: the 404 page, the letter strip in the list's own order, the top
// search box leaving the query on the search page, the book table beside the details pane
// (also with a column list saved by an older version), the author filter kept across visits,
// and user rows with a long name.

test.use({ viewport: { width: 1302, height: 800 } });

test('an unknown address shows the not-found page with the path and a way home', async ({ page }) => {
  await page.goto('/l/1/no-such-page');
  const card = page.getByTestId('page-not-found');
  await expect(card).toContainText('Page not found', { timeout: 15000 });
  await expect(page.getByTestId('not-found-path')).toHaveText('/l/1/no-such-page');
  await card.getByRole('link', { name: 'To the start page' }).click();
  await expect(page).toHaveURL(/\/l\/1\/home$/);
});

test('the letter strip reads in the order of the list', async ({ page, request }) => {
  const body = (await (await request.get('/api/v1/libraries/1/authors')).json()) as { letters: [string, number, number][] };
  const cyr = body.letters.map((l) => l[0]).filter((l) => /\p{Script=Cyrillic}/u.test(l));
  test.skip(cyr.length < 2, 'no Cyrillic names in the synthetic library');
  await page.goto('/l/1/authors');
  const scripts = page.getByRole('group', { name: 'Alphabet' });
  const strip = page.getByLabel('Jump to letter');
  await expect(strip).toBeVisible({ timeout: 15000 });
  if (await scripts.count()) await scripts.getByRole('button', { name: 'А–Я' }).click();
  const enabled = await strip.locator('button:not([disabled])').allTextContents();
  expect(enabled).toEqual(cyr);
});

test('the top search box shows the query only on its search page', async ({ page, request }) => {
  const author = await pickAuthor(request);
  const word = author.name.split(' ')[0];
  await page.goto(`/l/1/search?q=${encodeURIComponent(word)}`);
  const box = page.locator('.topbar input[role=combobox]');
  await expect(box).toHaveValue(word, { timeout: 15000 });
  await page.locator('.sidenav a[href="/l/1/new"]').click();
  await expect(box).toHaveValue('');
  await expect(page.locator('.sidenav [aria-current=page]')).toHaveText(/New arrivals/);
  await page.goBack();
  await expect(box).toHaveValue(word);
  await expect(page.locator('.sidenav [aria-current=page]')).toHaveCount(0);
});

test('the book table fits beside the details pane, also with an old saved column list', async ({ page, request }) => {
  const genre = await pickBiggestGenre(request);
  const overflow = () => page.locator('.scroll .vlist').evaluate((el) => el.scrollWidth - el.clientWidth);
  for (const cols of [null, ['author', 'series', 'genre', 'language', 'format', 'size', 'added']]) {
    await page.goto('/l/1/home');
    await page.evaluate((c) => fetch('/api/v1/me/prefs', {
      method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(c ? { 'cols.other': c } : {}),
    }), cols);
    await page.evaluate(() => localStorage.removeItem('freelib.prefs'));
    await page.goto(`/l/1/genres/${genre.id}`);
    await page.locator('.scroll .brow .title-btn').first().click({ timeout: 15000 });
    await expect(page.getByRole('complementary', { name: 'Details' })).toBeVisible();
    await expect.poll(overflow).toBeLessThanOrEqual(0);
    expect((await page.locator('.scroll .brow .title-cell').first().boundingBox())!.width).toBeGreaterThanOrEqual(219);
  }
  await page.evaluate(() => fetch('/api/v1/me/prefs', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: '{}' }));
});

test('the author filter is kept when coming back to Authors', async ({ page, request }) => {
  const author = await pickAuthor(request);
  await page.goto('/l/1/authors');
  const filter = page.locator('section.browser').getByLabel('Filter authors');
  await filter.fill(author.name);
  await page.getByRole('button', { name: author.name }).first().click();
  await expect(page).toHaveURL(new RegExp(`/authors/${author.id}`));
  await page.locator('.sidenav a[href="/l/1/new"]').click();
  await page.locator('.sidenav a[href="/l/1/authors"]').click();
  await expect(filter).toHaveValue(author.name);
});

test('a user row with a long name keeps its controls inside the card', async ({ page, request }) => {
  const name = `a-very-long-username-for-the-row-layout-${Date.now()}`;
  const r = await request.post('/api/v1/users', { data: { username: name, password: 'secret-password-1', role: 'reader' } });
  test.skip(!r.ok(), `cannot create users here (${r.status()})`);
  const user = (await r.json()) as { id: number };
  try {
    await request.patch(`/api/v1/users/${user.id}`, { data: { email: 'someone.with.a.long.address@example.org' } });
    await page.goto('/settings/users');
    const row = page.getByTestId('user-row').filter({ hasText: name });
    await expect(row).toBeVisible({ timeout: 15000 });
    const box = (await row.boundingBox())!;
    for (const el of await row.locator('input, select, button').all()) {
      const b = (await el.boundingBox())!;
      expect(b.x + b.width).toBeLessThanOrEqual(box.x + box.width + 0.5);
    }
    expect((await row.locator('input.email').boundingBox())!.width).toBeGreaterThanOrEqual(180);
  } finally {
    await request.delete(`/api/v1/users/${user.id}`);
  }
});
