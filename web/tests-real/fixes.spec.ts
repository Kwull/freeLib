import { test, expect } from '@playwright/test';
import { LIB } from './helpers';

// gen-inpx adds fixed "showcase" books (server/crates/import/src/synth.rs): Asimov's Foundation
// novels in several Russian translations under different titles and numbers of the series
// «Академия [Азимов]», and Marinina's two-volume «Люди за спиной». The importer joins the
// translations by series number; the details show the ISBN from the FB2 <publish-info>.
test.use({ viewport: { width: 2000, height: 836 }, colorScheme: 'dark' });

type ApiBook = { id: number; title: string; serno: number | null; series: { name: string } | null; editions?: { count: number; ids: number[] } };

async function asimov(request: import('@playwright/test').APIRequestContext) {
  const rows = (await (await request.get(`/api/v1/libraries/${LIB}/authors`)).json()).rows as [number, string, number][];
  const a = rows.find((r) => r[1] === 'Азимов Айзек')!;
  const books = (await (await request.get(`/api/v1/libraries/${LIB}/books?author=${a[0]}&group=true&limit=5000`)).json()).books as ApiBook[];
  return { id: a[0], books: books.filter((b) => b.series?.name === 'Академия [Азимов]') };
}

test('translations under other titles are one work; unnumbered books stay single', async ({ page, request }) => {
  const { id, books } = await asimov(request);
  const byNo = (n: number) => books.filter((b) => b.serno === n);
  // one row per number, with all its titles
  expect(byNo(6)).toHaveLength(1);
  expect(byNo(6)[0].editions?.count).toBeGreaterThanOrEqual(6);
  expect(byNo(7)[0].editions?.count).toBeGreaterThanOrEqual(5);
  expect(byNo(5)[0].editions?.count).toBe(2);
  expect(byNo(8)[0].editions).toBeUndefined();
  // the best copy is a plain novel, not the omnibus volume
  expect(byNo(6)[0].title).not.toMatch(/Книга \d/);
  for (const t of ['Академия. Книги 1-7', 'Академия. Начало', 'Путь к Академии', 'Миры Айзека Азимова. Книга 7']) {
    const b = books.find((x) => x.title === t);
    expect(b, t).toBeTruthy();
    expect(b!.editions, t).toBeUndefined();
  }

  // the list: the work's other titles as full rows
  const best = byNo(6)[0];
  await page.goto(`/l/${LIB}/authors/${id}?book=${best.id}`);
  const row = page.locator('.scroll .brow', { hasText: best.title }).first();
  await expect(row).toBeVisible({ timeout: 15000 });
  await row.getByTestId('editions-toggle').click();
  const eds = page.locator('.scroll').getByTestId('edition-row');
  await expect(eds).toHaveCount(best.editions!.count - 1);
  await expect(page.locator('.scroll').getByTestId('edition-title').filter({ hasText: 'Край Основания' })).toBeVisible();
  const boxes = (await page.locator('.scroll .vlist > div > div > *').evaluateAll((els) => els.map((e) => {
    const r = e.getBoundingClientRect();
    return [r.top, r.height];
  }))).sort((a, b) => a[0] - b[0]);
  for (let i = 1; i < boxes.length; i++) {
    expect(Math.round(boxes[i][1])).toBe(40);
    expect(Math.round(boxes[i][0] - boxes[i - 1][0])).toBe(40);
  }
});

test('volumes of one series number stay apart', async ({ request }) => {
  const rows = (await (await request.get(`/api/v1/libraries/${LIB}/authors`)).json()).rows as [number, string, number][];
  const m = rows.find((r) => r[1] === 'Маринина Александра Борисовна')!;
  const books = (await (await request.get(`/api/v1/libraries/${LIB}/books?author=${m[0]}&group=true&limit=5000`)).json()).books as ApiBook[];
  const v1 = books.filter((b) => /^Люди за спиной[.,] том 1$/i.test(b.title));
  const v2 = books.filter((b) => b.title === 'Люди за спиной. Том 2');
  expect(v1).toHaveLength(1);
  expect(v1[0].editions?.count).toBe(2);
  expect(v2).toHaveLength(1);
  expect(v2[0].editions).toBeUndefined();
});

test('the details show the ISBN, publisher and year', async ({ page, request }) => {
  const { id, books } = await asimov(request);
  let found: { id: number; isbn13: string } | null = null;
  for (const b of books) {
    for (const eid of b.editions?.ids ?? [b.id]) {
      const d = await (await request.get(`/api/v1/libraries/${LIB}/books/${eid}`)).json();
      if (d.isbn?.length) { found = { id: eid, isbn13: d.isbn[0].isbn13 }; break; }
    }
    if (found) break;
  }
  expect(found).toBeTruthy();
  expect(['9785699120147', '9785170123452']).toContain(found!.isbn13);
  await page.goto(`/l/${LIB}/authors/${id}?book=${found!.id}`);
  const details = page.getByRole('complementary', { name: 'Details' });
  await expect(details.getByTestId('isbn')).toContainText(/978/, { timeout: 15000 });
  await expect(details.getByTestId('isbn').getByRole('button', { name: `Copy ISBN ${found!.isbn13}` })).toBeVisible();
  await expect(details.getByTestId('publisher')).toContainText(/(Эксмо|АСТ), 20\d\d/);
});
