import { test, expect, type APIRequestContext } from '@playwright/test';
import { LIB } from './helpers';

// gen-inpx's search / count showcase (lib ids 200..): «Азимов Айзек» with his books in the
// series «FANTASTIKAS PASAULĖ» and «Fondation», names near his (Асимова Мария, Агаев Рамин
// Назимович), a Бережной essay about Азимов and robots, and «Сами боги» with one edition in a
// series and the best copy outside it.
test.use({ viewport: { width: 1440, height: 900 } });

async function asimov(request: APIRequestContext): Promise<[number, string, number]> {
  const rows = (await (await request.get(`/api/v1/libraries/${LIB}/authors`)).json()).rows as [number, string, number][];
  const row = rows.find((r) => r[1] === 'Азимов Айзек');
  if (!row) throw new Error('no Азимов Айзек in the synthetic library');
  return row;
}

type Row = { id: number; title: string; series: { id: number; name: string } | null; authors: { name: string }[] };

test('one count model: header = list = sidebar, with and without anthologies', async ({ page, request }) => {
  const [id, , listed] = await asimov(request);
  const summary = await (await request.get(`/api/v1/libraries/${LIB}/authors/${id}/summary`)).json();
  const list = await (await request.get(`/api/v1/libraries/${LIB}/books?author=${id}&group=1&limit=5000`)).json();
  const rows = list.books as Row[];
  // works everywhere: the name list, the summary (header) and the grouped list
  expect(summary.count).toBe(listed);
  expect(list.total).toBe(summary.count);
  expect(summary.files).toBeGreaterThan(summary.count);
  const outside = rows.filter((b) => !b.series);
  expect(outside.length).toBe(summary.withoutSeries);
  // "Outside series" rows come after every series
  const firstOutside = rows.findIndex((b) => !b.series);
  expect(rows.slice(firstOutside).every((b) => !b.series)).toBe(true);
  for (const s of summary.series as { id: number; count: number }[]) {
    expect(rows.filter((b) => b.series?.id === s.id).length).toBe(s.count);
  }

  await page.goto(`/l/${LIB}/authors/${id}`);
  const header = page.locator('.scope-header');
  await expect(header.getByTestId('header-count')).toHaveText(`${summary.count} books`);
  await expect(header.getByTestId('header-files')).toContainText(`${summary.files} files`);
  const details = page.getByRole('complementary', { name: 'Details' });
  await expect(details.getByTestId('summary-outside').locator('.n')).toHaveText(String(summary.withoutSeries));
  // the last group of the list is "Outside series", with the same count
  const lastGroup = async () => {
    await page.locator('.scroll .vlist').evaluate((el) => { el.scrollTop = el.scrollHeight; });
    return page.locator('.scroll .group-head').last();
  };
  await expect(async () => {
    const g = await lastGroup();
    await expect(g.locator('.gname')).toHaveText('Outside series', { timeout: 1000 });
    await expect(g.locator('.gcount')).toHaveText(String(summary.withoutSeries), { timeout: 1000 });
  }).toPass({ timeout: 15000 });

  // without anthologies: the list, its groups and the sidebar drop the same works
  await page.getByRole('button', { name: /Hide anthologies/ }).click();
  const noAnth = summary.withoutSeries - summary.withoutSeriesAnthologies;
  await expect(details.getByTestId('summary-outside').locator('.n')).toHaveText(String(noAnth));
  await expect(page.locator('.toolbar .shown')).toHaveText(`${summary.count - summary.anthologies} of ${summary.count}`);
  await expect(async () => {
    const g = await lastGroup();
    await expect(g.locator('.gcount')).toHaveText(String(noAnth), { timeout: 1000 });
  }).toPass({ timeout: 15000 });
  await request.put('/api/v1/me/prefs', { data: {} });
});

test('genre and series: tree / list count = list total', async ({ request }) => {
  const genres = (await (await request.get(`/api/v1/libraries/${LIB}/genres`)).json()) as { id: number; parent: number; count: number }[];
  const top = genres.filter((g) => g.parent === 0).reduce((a, b) => (b.count > a.count ? b : a));
  const leaf = genres.filter((g) => g.parent !== 0).reduce((a, b) => (b.count > a.count ? b : a));
  for (const g of [top, leaf]) {
    const r = await (await request.get(`/api/v1/libraries/${LIB}/books?genre=${g.id}&group=1&limit=1`)).json();
    expect(r.total).toBe(g.count);
  }
  const series = (await (await request.get(`/api/v1/libraries/${LIB}/series`)).json()).rows as [number, string, number][];
  for (const name of ['Fondation', 'FANTASTIKAS PASAULĖ', 'Академия [Азимов]']) {
    const s = series.find((r) => r[1] === name)!;
    const r = await (await request.get(`/api/v1/libraries/${LIB}/books?series=${s[0]}&group=1&limit=1`)).json();
    expect(r.total, name).toBe(s[2]);
  }
  // search facets count the rows, like the results
  const res = await (await request.get(`/api/v1/libraries/${LIB}/search?q=${encodeURIComponent('азимов')}&group=1&limit=1`)).json();
  for (const [ext, n] of res.facets.ext as [string, number][]) {
    const f = await (await request.get(`/api/v1/libraries/${LIB}/search?q=${encodeURIComponent('азимов')}&group=1&limit=1&ext=${ext}`)).json();
    expect(f.total, ext).toBe(n);
  }
});

test('Latin spellings find Азимов; names rank by the last name', async ({ page, request }) => {
  const search = async (q: string, extra = '') =>
    (await request.get(`/api/v1/libraries/${LIB}/search?q=${encodeURIComponent(q)}&group=1${extra}`)).json();
  for (const q of ['asimov', 'azimov', 'asimow', 'azimoff']) {
    expect((await search(q, '&kind=authors')).authors[0]?.name, q).toBe('Азимов Айзек');
  }
  const osn = await search('asimov osnovanie');
  expect((osn.books as Row[]).some((b) => b.title.startsWith('Основание'))).toBe(true);
  const robots = await search('азимоф роботы');
  expect((robots.books as Row[])[0].authors[0].name).toBe('Азимов Айзек');

  // the typeahead: Азимов first
  await page.goto(`/l/${LIB}/authors`);
  await page.getByRole('combobox').fill('asimov');
  await expect(page.getByRole('option').first()).toContainText('Азимов Айзек');

  // the author filter: the last name first, infix matches last; Latin spellings work too
  const filter = page.getByLabel('Filter authors');
  await filter.fill('азимов');
  const names = page.locator('section.browser .arow .name');
  await expect(names.first()).toHaveText('Азимов Айзек');
  await expect(page.locator('section.browser .arow', { hasText: 'Агаев Рамин Назимович' })).toBeVisible();
  await filter.fill('asimov');
  await expect(names.first()).toHaveText('Азимов Айзек', { timeout: 15000 });
});

test('start page: "Well-rated new arrivals" stays after a download', async ({ page, request }) => {
  const list = await (await request.get(`/api/v1/libraries/${LIB}/books?since=1900-01-01&ext=fb2&limit=1`)).json();
  const r = await request.get(`/api/v1/libraries/${LIB}/books/${list.books[0].id}/file`);
  expect(r.ok()).toBe(true);
  const home = await (await request.get(`/api/v1/libraries/${LIB}/home`)).json();
  expect(home.empty).toBe(false);
  expect(home.picks.length).toBeGreaterThan(0);
  await page.goto(`/l/${LIB}`);
  await expect(page.getByTestId('home-picks')).toBeVisible({ timeout: 15000 });
});

// gen-inpx's ranking showcase (lib ids 300..) and one definition of the library counts
test('«пикник» ranks the Strugatskys first, nonsense finds nothing, library counts agree', async ({ page, request }) => {
  const r = await (await request.get(`/api/v1/libraries/${LIB}/search?q=${encodeURIComponent('пикник')}&group=1`)).json();
  const rows = (r.books as Row[]).map((b) => [b.title, b.authors[0].name]);
  const roadside = rows.findIndex(([t, a]) => t.startsWith('Пикник на обочине') && a.startsWith('Стругацкий'));
  expect(roadside, JSON.stringify(rows.slice(0, 5))).toBeGreaterThanOrEqual(0);
  expect(roadside).toBeLessThan(3);
  expect(rows.filter(([t, a]) => t === 'Пикник' && a === 'Автор неизвестен').length).toBe(2);

  const z = await (await request.get(`/api/v1/libraries/${LIB}/search?q=zzzqxw`)).json();
  expect(z.corrected ?? null).toBeNull();
  expect(z.didYouMean ?? null).toBeNull();
  expect(z.total).toBe(0);
  await page.goto(`/l/${LIB}/search?q=zzzqxw`);
  await expect(page.getByText('Nothing found')).toBeVisible();

  const lib = (await (await request.get('/api/v1/libraries')).json()).find((l: { id: number }) => l.id === LIB);
  const rowsOf = async (kind: string) => ((await (await request.get(`/api/v1/libraries/${LIB}/${kind}`)).json()).rows as unknown[]).length;
  expect(lib.authorCount).toBe(await rowsOf('authors'));
  expect(lib.seriesCount).toBe(await rowsOf('series'));
  const all = await (await request.get(`/api/v1/libraries/${LIB}/books?since=1900-01-01&group=1&limit=1`)).json();
  expect(all.total).toBe(lib.workCount);
  const jobs = (await (await request.get('/api/v1/jobs')).json()) as { kind: string; message: string }[];
  const f = (n: number) => n.toLocaleString('en-US');
  const imp = jobs.find((j) => j.kind === 'import');
  if (imp) expect(imp.message).toContain(`${f(lib.bookCount)} books (${f(lib.workCount)} works) · ${f(lib.authorCount)} authors · ${f(lib.seriesCount)} series`);
});
