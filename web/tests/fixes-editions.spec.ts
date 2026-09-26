import { test, expect, devices, type Page, type Locator } from '@playwright/test';

// Editions and series groups in the series-grouped (virtualised) book list: full-height rows
// that never overlap, in the table, the grid and the phone list. The mock author
// "Азимов Айзек" has the Foundation novels of «Академия [Азимов]» in several translations
// under different titles and numbers (docs/web/ARCHITECTURE.md, "Editions").

async function openAuthor(page: Page, name: string) {
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill(name);
  await page.getByRole('button', { name: new RegExp(name) }).first().click();
  await expect(page.getByRole('heading', { level: 1, name })).toBeVisible();
}

// the iPhone 13's viewport, touch and UA, rendered by chromium (the only browser installed)
const { defaultBrowserType: _b, ...iphone } = devices['iPhone 13'];

type Box = { top: number; bottom: number; height: number; left: number; right: number };

/** Boxes of the rendered rows of a virtual list, top to bottom. */
async function rowBoxes(rows: Locator): Promise<Box[]> {
  const boxes = await rows.evaluateAll((els) => els.map((e) => {
    const r = e.getBoundingClientRect();
    return { top: r.top, bottom: r.bottom, height: r.height, left: r.left, right: r.right };
  }));
  return boxes.sort((a, b) => a.top - b.top);
}

function expectStacked(boxes: Box[], height: number) {
  expect(boxes.length).toBeGreaterThan(1);
  for (const b of boxes) expect(Math.round(b.height)).toBe(height);
  for (let i = 1; i < boxes.length; i++) {
    // each row starts where the previous one ends: no overlap, no gap
    expect(Math.round(boxes[i].top - boxes[i - 1].top)).toBe(height);
  }
}

/** Every child cell of a row sits inside the row (nothing wraps onto a second grid line). */
async function expectCellsInside(row: Locator) {
  const res = await row.evaluate((el) => {
    const r = el.getBoundingClientRect();
    return [...el.children].map((c) => {
      const b = c.getBoundingClientRect();
      return b.height === 0 || (b.top >= r.top - 0.5 && b.bottom <= r.bottom + 0.5);
    });
  });
  expect(res.every(Boolean)).toBe(true);
}

test.describe('desktop', () => {
  test.use({ viewport: { width: 2000, height: 836 }, colorScheme: 'dark' });

  test('expanded editions are full rows on the table grid, with their own titles', async ({ page }) => {
    await openAuthor(page, 'Азимов Айзек');
    const list = page.locator('.scroll');
    // #6: four titles, three copies of one of them → one work
    const row6 = list.locator('.brow:not(.edition-row)', { hasText: /Академия на краю гибели|Край Основания|Сообщество на краю/ }).first();
    await expect(row6.getByTestId('editions-toggle')).toHaveText('6 editions');
    await row6.getByTestId('editions-toggle').click();
    const eds = list.getByTestId('edition-row');
    await expect(eds).toHaveCount(5);
    // translations under other titles show their title; copies of the group title do not
    await expect(list.getByTestId('edition-title').filter({ hasText: 'Край Основания' })).toBeVisible();
    await expect(list.getByTestId('edition-title').filter({ hasText: 'Сообщество на краю' })).toBeVisible();
    await expect(list.getByTestId('edition-title').filter({ hasText: 'Миры Айзека Азимова. Книга 9' })).toBeVisible();

    // geometry: 40px rows, stacked, the checkbox vertically inside its row
    expectStacked(await rowBoxes(list.locator('.vlist > div > div > *')), 40);
    for (const ed of await eds.all()) {
      await expectCellsInside(ed);
      const [r, c] = await Promise.all([ed.boundingBox(), ed.locator('input[type=checkbox]').boundingBox()]);
      expect(c!.y).toBeGreaterThan(r!.y);
      expect(c!.y + c!.height).toBeLessThan(r!.y + r!.height);
      expect(Math.abs(c!.y + c!.height / 2 - (r!.y + r!.height / 2))).toBeLessThan(2);
    }
    // the edition row shares the grid template (the Size column lines up with the header)
    const head = await page.locator('.brow.head .hcell', { hasText: 'Size' }).boundingBox();
    const cell = await eds.first().locator('.right').first().boundingBox();
    expect(Math.abs(head!.x + head!.width - (cell!.x + cell!.width))).toBeLessThan(1.5);

    // #7 and #5 are joined too; the unnumbered omnibus volumes stay single
    await expect(list.locator('.brow', { hasText: /Академия и Земля|Основание и Земля/ }).first().getByTestId('editions-toggle')).toHaveText('5 editions');
    for (const t of ['Академия. Начало', 'Академия. Первая трилогия', 'Путь к Академии', 'Миры Айзека Азимова. Книга 7']) {
      const r = list.locator('.brow:not(.edition-row)', { hasText: t });
      await expect(r).toHaveCount(1);
      await expect(r.getByTestId('editions-toggle')).toHaveCount(0);
    }

    // collapsing again removes the rows and keeps the stack intact
    await row6.getByTestId('editions-toggle').click();
    await expect(eds).toHaveCount(0);
    expectStacked(await rowBoxes(list.locator('.vlist > div > div > *')), 40);
    await page.screenshot({ path: 'test-results/screenshots/fixes-editions.png' });
  });

  test('volumes of one number stay apart; copies of one volume join', async ({ page }) => {
    await openAuthor(page, 'Маринина Александра');
    const list = page.locator('.scroll');
    const vol1 = list.locator('.brow', { hasText: /Люди за спиной[.,] том 1/i }).first();
    await expect(vol1.getByTestId('editions-toggle')).toHaveText('2 editions');
    const vol2 = list.locator('.brow', { hasText: 'Люди за спиной. Том 2' });
    await expect(vol2).toHaveCount(1);
    await expect(vol2.getByTestId('editions-toggle')).toHaveCount(0);
  });

  test('series groups open, close, collapse all and scroll with consistent rows', async ({ page }) => {
    // "Asimov Isaac": ~20 series, so the groups start collapsed
    await openAuthor(page, 'Asimov Isaac');
    const list = page.locator('.scroll');
    const vl = list.locator('.vlist');
    const rows = list.locator('.vlist > div > div > *');
    await expect(list.locator('.brow')).toHaveCount(0);
    // open a group further down, after scrolling
    await vl.evaluate((e) => (e.scrollTop = 200));
    const g = list.locator('.group-head').nth(10);
    const count = Number(await g.locator('.gcount').innerText());
    await g.locator('.gtoggle').click();
    await expect(g.locator('.gtoggle')).toHaveAttribute('aria-expanded', 'true');
    await expect(list.locator('.brow')).toHaveCount(count);
    expectStacked(await rowBoxes(rows), 40);
    // open editions inside it, if any, and everything still stacks
    const tag = list.getByTestId('editions-toggle').first();
    if (await tag.count()) { await tag.click(); await expect(list.getByTestId('edition-row').first()).toBeVisible(); }
    expectStacked(await rowBoxes(rows), 40);
    // collapse all → expand all → scroll to the end → the last rows are reachable and stacked
    await page.getByRole('button', { name: 'Collapse all' }).click();
    await expect(list.locator('.brow')).toHaveCount(0);
    await page.getByRole('button', { name: 'Expand all' }).click();
    await expect(list.locator('.brow').first()).toBeVisible();
    await vl.evaluate((e) => (e.scrollTop = e.scrollHeight));
    await page.waitForTimeout(100);
    expectStacked(await rowBoxes(rows), 40);
    const last = (await rowBoxes(rows)).at(-1)!;
    const vbox = (await vl.boundingBox())!;
    expect(last.bottom).toBeGreaterThan(vbox.y + vbox.height - 41);
    // collapse all while scrolled to the end: the list shows its group rows, not a blank area
    await page.getByRole('button', { name: 'Collapse all' }).click();
    await expect(list.locator('.brow')).toHaveCount(0);
    const boxes = await rowBoxes(rows);
    expectStacked(boxes, 40);
    expect(boxes.some((b) => b.top >= vbox.y && b.bottom <= vbox.y + vbox.height)).toBe(true);
    expect(await vl.evaluate((e) => e.scrollTop <= e.scrollHeight - e.clientHeight)).toBe(true);
  });

  test('the cover grid shows the series groups and their editions', async ({ page }) => {
    await openAuthor(page, 'Азимов Айзек');
    await page.getByRole('button', { name: 'Cover view' }).click();
    const grid = page.locator('.grid-view');
    const head = grid.locator('.grid-group', { hasText: 'Академия [Азимов]' });
    await expect(head).toBeVisible();
    await expect(grid.locator('.cover-card .ed-count', { hasText: '6 editions' })).toBeVisible();
    const cards = grid.locator('.cover-card');
    const n = await cards.count();
    await head.locator('.gtoggle').click();
    await expect(head.locator('.gtoggle')).toHaveAttribute('aria-expanded', 'false');
    await expect(cards).not.toHaveCount(n);
    await head.locator('.gtoggle').click();
    await expect(cards).toHaveCount(n);
    await page.getByRole('button', { name: 'Table view' }).click();
  });
});

test.describe('phone', () => {
  test.use({ ...iphone, colorScheme: 'dark' });

  test('phone list: edition rows are full 64px rows', async ({ page }) => {
    await page.goto('/l/1/authors');
    await page.getByLabel('Filter authors').fill('Азимов Айзек');
    await page.getByRole('button', { name: /Азимов Айзек/ }).first().click();
    const list = page.locator('.mobile-list');
    const row = list.locator('.m-row', { hasText: /Академия на краю гибели|Край Основания/ }).first();
    await row.getByTestId('editions-toggle').click();
    await expect(list.getByTestId('edition-row')).toHaveCount(5);
    await expect(list.getByTestId('edition-title').filter({ hasText: 'Край Основания' })).toBeVisible();
    expectStacked(await rowBoxes(list.locator('.vlist > div > div > *')), 64);
    for (const ed of await list.getByTestId('edition-row').all()) await expectCellsInside(ed);
    await page.screenshot({ path: 'test-results/screenshots/fixes-phone-editions.png' });
  });
});
