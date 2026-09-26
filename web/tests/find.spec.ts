import { test, expect, type Page } from '@playwright/test';

// Start page, follow, did-you-mean / highlighting and editions against the mock
// (web/mock/find.ts: the mock user has read two books of «Мир Полудня» and «Я, робот»).

async function authorId(page: Page, name: string): Promise<number> {
  const r = await page.request.get(`/api/v1/libraries/1/search?q=${encodeURIComponent(name)}&kind=authors`);
  const body = await r.json();
  return body.authors.find((a: { name: string }) => a.name === name).id;
}

test('start page is the landing page: continue series, new from authors, dismiss', async ({ page }) => {
  await page.goto('/');
  await expect(page).toHaveURL(/\/l\/1\/home$/);
  await expect(page.getByRole('heading', { name: 'What to read next' })).toBeVisible();
  const cont = page.getByTestId('home-continue');
  const noon = cont.getByTestId('continue-series').filter({ hasText: 'Мир Полудня' });
  await expect(noon).toContainText('Трудно быть богом');
  await expect(noon).toContainText('Next: #3');
  await expect(noon).toContainText('2 of 5 done');
  // new books by the authors read: Strugatsky and Asimov, not the ones already read
  const fresh = page.getByTestId('home-new');
  await expect(fresh).toContainText('Конец Вечности');
  await expect(fresh).toContainText('You read');
  await expect(fresh).not.toContainText('Попытка к бегству');
  // the window can be changed
  await fresh.getByRole('button', { name: '7 days' }).click();
  await expect(fresh.getByRole('button', { name: '7 days' })).toHaveAttribute('aria-pressed', 'true');
  await fresh.getByRole('button', { name: 'Since last visit' }).click();
  // New arrivals and the browse lists stay one click away
  await expect(page.getByRole('navigation', { name: 'Browse' }).getByRole('link', { name: 'New arrivals' })).toBeVisible();

  // one-click send to the default device
  await noon.getByTestId('home-send').click();
  await expect(page.locator('.toast').first()).toBeVisible();

  // not interested: the series disappears (restored afterwards for the other tests)
  const robots = cont.getByTestId('continue-series').filter({ hasText: 'Роботы' });
  await expect(robots).toBeVisible();
  await robots.getByTestId('dismiss-series').click();
  await expect(robots).toHaveCount(0);
  const r = await page.request.get('/api/v1/libraries/1/search?q=Роботы&kind=series');
  const sid = (await r.json()).series.find((s: { name: string }) => s.name === 'Роботы').id;
  await page.request.post('/api/v1/libraries/1/home/dismiss', { data: { series: sid, dismissed: false } });
});

test('start page for a new user explains itself and offers new books', async ({ page, context }) => {
  await context.addCookies([{ name: 'freelib_mock', value: encodeURIComponent('s=newuser'), url: 'http://localhost:5183' }]);
  await page.goto('/l/1/home');
  await expect(page.getByTestId('home-empty')).toContainText('fills up as you read');
  await expect(page.getByTestId('home-picks').getByTestId('home-book').first()).toBeVisible();
  await expect(page.getByTestId('home-continue')).toHaveCount(0);
});

test('follow an author from the author page', async ({ page }) => {
  const id = await authorId(page, 'Азимов Айзек');
  await page.goto(`/l/1/authors/${id}`);
  const btn = page.getByTestId('follow-author').first();
  await expect(btn).toHaveText(/Follow/);
  await btn.click();
  await expect(btn).toHaveAttribute('aria-pressed', 'true');
  await expect(btn).toHaveText(/Following/);
  const f = await (await page.request.get('/api/v1/libraries/1/follows')).json();
  expect(f.authors.map((a: { id: number }) => a.id)).toContain(id);
  await page.goto('/l/1/home');
  await expect(page.getByTestId('home-new')).toContainText('You follow Азимов Айзек');
  // stop following again
  await page.goto(`/l/1/authors/${id}`);
  await page.getByTestId('follow-author').first().click();
  await expect(page.getByTestId('follow-author').first()).toHaveAttribute('aria-pressed', 'false');
});

test('search: word forms, transliteration, typo correction and did-you-mean', async ({ page }) => {
  // a word form highlights the words it matched
  await page.goto('/l/1/search?q=книгу');
  const row = page.getByTestId('search-book').filter({ hasText: 'Книга о книгах' });
  await expect(row.locator('.title mark')).toHaveText(['Книга', 'книгах']);

  // Latin → Cyrillic
  await page.goto('/l/1/search?q=strugatsky');
  await expect(page.locator('.series-card', { hasText: 'Стругацкий Аркадий Натанович' }).locator('mark')).toHaveText('Стругацкий');

  // nothing found as typed → corrected
  await page.goto('/l/1/search?q=Сругацкий');
  await expect(page.getByTestId('search-corrected')).toContainText('стругацкий');
  await expect(page.locator('.series-card', { hasText: 'Стругацкий Аркадий' }).first()).toBeVisible();

  // little found and no author → the corrected results, with a way back to the query as typed
  await page.goto('/l/1/search?q=азимв');
  await expect(page.getByTestId('search-corrected')).toContainText('Showing results for азимов');
  await expect(page.locator('.series-card', { hasText: 'Азимов Айзек' }).locator('mark')).toHaveText('Азимов');
  await page.getByTestId('search-exact').click();
  await expect(page).toHaveURL(/exact=1/);
  await expect(page.getByTestId('search-book')).toHaveCount(1);
  await expect(page.getByTestId('search-book')).toContainText('Записки на полях');
  await expect(page.getByTestId('search-corrected')).toHaveCount(0);

  // the typeahead shows the corrected results too
  await page.goto('/l/1/authors');
  await page.getByRole('combobox').fill('азимв');
  await expect(page.getByTestId('typeahead-corrected')).toContainText('азимов');
  await expect(page.getByRole('option', { name: /Азимов Айзек/ }).first()).toBeVisible();
});

test('editions: one row per work, expand, pick another, grouping switch', async ({ page }) => {
  await page.goto('/l/1/search?q=пикник обочине');
  const rows = page.getByTestId('search-book').filter({ hasText: 'Пикник на обочине' });
  await expect(rows).toHaveCount(1);
  const toggle = rows.getByTestId('editions-toggle');
  await expect(toggle).toHaveText(/4 editions/);
  await toggle.click();
  const list = page.getByTestId('editions-list');
  await expect(list.getByRole('listitem')).toHaveCount(4);
  await expect(list.getByRole('listitem').first()).toContainText('best copy');
  await expect(list).toContainText('другой перевод');
  await expect(list).toContainText('EPUB');

  // the author's list: the grouped row expands into its other editions
  const id = await authorId(page, 'Стругацкий Аркадий Натанович');
  await page.goto(`/l/1/authors/${id}`);
  const pr = page.locator('.scroll .brow', { hasText: 'Пикник на обочине' });
  await expect(pr).toHaveCount(1);
  await pr.getByTestId('editions-toggle').click();
  await expect(page.getByTestId('edition-row')).toHaveCount(3);
  await page.getByTestId('edition-row').first().click();
  await expect(page.locator('.details')).toContainText('Пикник на обочине');

  // grouping off: every edition is its own row (and back on)
  await page.getByRole('button', { name: /Filter/ }).click();
  await page.getByTestId('group-editions').uncheck();
  await expect(page.locator('.scroll .brow', { hasText: 'Пикник на обочине' })).toHaveCount(4);
  await page.getByTestId('group-editions').check();
  await expect(page.locator('.scroll .brow', { hasText: 'Пикник на обочине' })).toHaveCount(1);
});
