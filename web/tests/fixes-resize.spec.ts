import { test, expect, type Page, type Locator } from '@playwright/test';

// Pane splitters and column handles: pointer capture (the drag survives leaving the handle and
// the window), limits, a squeezed pane without a dead zone, touch/pen pointers, bad stored
// widths, and columns that stay in line with the header.
test.use({ viewport: { width: 1600, height: 900 }, colorScheme: 'dark' });

async function openDoyle(page: Page) {
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
  await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
  await expect(page.getByText('The Hound of the Baskervilles').first()).toBeVisible({ timeout: 15000 });
}

async function center(l: Locator) {
  const b = (await l.boundingBox())!;
  return { x: b.x + b.width / 2, y: b.y + Math.min(b.height / 2, 200) };
}
const width = async (l: Locator) => (await l.boundingBox())!.width;

test('drag past the limits clamps; releasing outside the window ends the drag', async ({ page }) => {
  await openDoyle(page);
  const list = page.locator('section.browser');
  const handle = page.getByRole('separator', { name: 'Resize the list' });
  const { x, y } = await center(handle);
  await page.mouse.move(x, y);
  await page.mouse.down();
  // far past the maximum, over the book table and the details pane
  await page.mouse.move(x + 900, y, { steps: 8 });
  await expect.poll(() => width(list)).toBe(520);
  // leave the window and release there
  await page.mouse.move(x + 900, -40, { steps: 2 });
  await page.mouse.up();
  await expect(page.locator('body.col-resizing')).toHaveCount(0);
  await expect(handle).toHaveAttribute('aria-valuenow', '520');
  // the drag is over: moving the mouse does not resize any more
  await page.mouse.move(x - 200, y, { steps: 4 });
  expect(await width(list)).toBe(520);
  // past the minimum
  const h2 = await center(handle);
  await page.mouse.move(h2.x, h2.y);
  await page.mouse.down();
  await page.mouse.move(h2.x - 900, h2.y, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => width(list)).toBe(200);
  await expect(handle).toHaveAttribute('aria-valuenow', '200');
  // double click resets
  await handle.dblclick();
  await expect.poll(() => width(list)).toBe(280);
});

test('a squeezed details pane resizes from what is on screen (no dead zone)', async ({ page }) => {
  await page.setViewportSize({ width: 1100, height: 800 });
  await openDoyle(page);
  await page.getByText('The Hound of the Baskervilles').first().click();
  const details = page.getByRole('complementary', { name: 'Details' });
  const handle = page.getByRole('separator', { name: 'Resize the details pane' });
  // store the maximum width: the window cannot show it
  await handle.focus();
  await page.keyboard.press('End');
  await expect(handle).toHaveAttribute('aria-valuenow', '640');
  const shown = await width(details);
  expect(shown).toBeLessThan(600);
  // dragging right by 40px narrows it by 40px at once
  const { x, y } = await center(handle);
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 40, y, { steps: 4 });
  await expect.poll(() => width(details)).toBeLessThan(shown - 30);
  await page.mouse.up();
  // the stored width is what the layout gave, not the unreachable maximum
  await expect.poll(async () => Number(await handle.getAttribute('aria-valuenow'))).toBeLessThan(shown);
});

test('touch and pen pointers drag the splitter', async ({ page }) => {
  await openDoyle(page);
  const list = page.locator('section.browser');
  const handle = page.getByRole('separator', { name: 'Resize the list' });
  for (const [pointerType, dx] of [['touch', 60], ['pen', -40]] as const) {
    const w0 = await width(list);
    const { x, y } = await center(handle);
    const ev = (clientX: number) => ({ pointerId: 7, pointerType, isPrimary: true, button: 0, buttons: 1, clientX, clientY: y, bubbles: true });
    await handle.dispatchEvent('pointerdown', ev(x));
    await handle.dispatchEvent('pointermove', ev(x + dx / 2));
    await handle.dispatchEvent('pointermove', ev(x + dx));
    await handle.dispatchEvent('pointerup', { ...ev(x + dx), buttons: 0 });
    await expect.poll(() => width(list)).toBe(Math.round(w0 + dx));
  }
});

test('bad stored widths are clamped on load', async ({ page }) => {
  // an old / hand-edited prefs copy (the server has none for this browser)
  await page.addInitScript(() => {
    if (sessionStorage.getItem('seeded')) return;
    sessionStorage.setItem('seeded', '1');
    localStorage.setItem('freelib.prefs', JSON.stringify({
      panes: { list: 5000, details: 'abc', nav: -3 }, columns: { size: 9999, added: null, genre: '100' },
    }));
  });
  await openDoyle(page);
  await expect(page.getByRole('separator', { name: 'Resize the list' })).toHaveAttribute('aria-valuenow', '520');
  await expect(page.getByRole('separator', { name: 'Resize the navigation' })).toHaveAttribute('aria-valuenow', '168');
  await expect(page.getByRole('separator', { name: 'Resize column Size' })).toHaveAttribute('aria-valuenow', '140');
  await expect(page.getByRole('separator', { name: 'Resize column Added' })).toHaveAttribute('aria-valuenow', '92');
  await page.getByText('The Hound of the Baskervilles').first().click();
  await expect(page.getByRole('separator', { name: 'Resize the details pane' })).toHaveAttribute('aria-valuenow', '360');
});

test('column widths apply to header and rows, also after scrolling and toggling columns', async ({ page }) => {
  await openDoyle(page);
  const aligned = async () => {
    const head = await page.locator('.brow.head .hcell', { hasText: 'Size' }).boundingBox();
    const row = page.locator('.scroll .brow').nth(3);
    const idx = await page.locator('.brow.head > *').evaluateAll((els) => els.findIndex((e) => e.textContent?.trim() === 'Size'));
    const cell = await row.locator(':scope > *').nth(idx).boundingBox();
    return Math.abs(head!.x + head!.width - (cell!.x + cell!.width));
  };
  const handle = page.getByRole('separator', { name: 'Resize column Size' });
  const { x, y } = await center(handle);
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x - 50, y, { steps: 5 });
  // live: rows follow while dragging
  expect(await aligned()).toBeLessThan(1.5);
  // release outside the window
  await page.mouse.move(x - 50, 2000, { steps: 2 });
  await page.mouse.up();
  await expect(handle).toHaveAttribute('aria-valuenow', /^1[12]\d$/);
  expect(await aligned()).toBeLessThan(1.5);
  // scroll, add columns so the table scrolls sideways, scroll sideways: header follows
  await page.locator('.scroll .vlist').evaluate((e) => (e.scrollTop = 300));
  await page.getByRole('button', { name: 'Columns' }).click();
  for (const c of ['Author', 'Series', 'Genre', 'Language', 'Format']) await page.getByTestId('columns-menu').getByLabel(c).check();
  await page.keyboard.press('Escape');
  await page.locator('.scroll .vlist').evaluate((e) => (e.scrollLeft = 10_000));
  await expect.poll(async () => {
    const [h, r] = await Promise.all([
      page.locator('.head-wrap').evaluate((e) => e.scrollLeft),
      page.locator('.scroll .vlist').evaluate((e) => e.scrollLeft),
    ]);
    return h === r && r > 0;
  }).toBe(true);
  expect(await aligned()).toBeLessThan(1.5);
  // narrowing a column while scrolled to the right edge keeps them in step
  const genre = page.getByRole('separator', { name: 'Resize column Genre' });
  await genre.focus();
  await page.keyboard.press('Shift+ArrowRight');
  await expect.poll(async () => {
    const [h, r] = await Promise.all([
      page.locator('.head-wrap').evaluate((e) => e.scrollLeft),
      page.locator('.scroll .vlist').evaluate((e) => e.scrollLeft),
    ]);
    return h === r;
  }).toBe(true);
  expect(await aligned()).toBeLessThan(1.5);
  // restore
  await page.getByRole('button', { name: 'Columns' }).click();
  for (const c of ['Author', 'Series', 'Genre', 'Language', 'Format']) await page.getByTestId('columns-menu').getByLabel(c).uncheck();
  await page.keyboard.press('Escape');
  await page.evaluate(() => fetch('/api/v1/me/prefs', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: '{}' }));
});
