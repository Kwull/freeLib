import { test, expect, devices, type Page, type Locator } from '@playwright/test';

// Fixes from testing the live deployment: menus that are never clipped, dialog footers on
// phones, the Follow toast, bad links, columns that fit, search filters in the URL, the cover
// grid, long annotation URLs, the reader's position and clean-up, keyboard lists, focus return
// from dialogs, the phone letter strip and folder devices without a folder.

const { defaultBrowserType: _b, ...iphone } = devices['iPhone 13'];
const SHOTS = '../docs/web/screenshots';

async function openAuthor(page: Page, name = 'Doyle Arthur Conan') {
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill(name);
  await page.getByRole('button', { name: new RegExp(name) }).first().click();
  await expect(page.locator('.scroll [role=row], .m-row, .m-group').first()).toBeVisible({ timeout: 15000 });
}

async function openHound(page: Page) {
  await openAuthor(page);
  await page.getByText('The Hound of the Baskervilles').first().click();
  await expect(page.getByRole('button', { name: 'Download as' })).toBeVisible();
}

/** The popup is fully inside the viewport and nothing covers it: the element at the centre
 *  of each of its items (or of itself) belongs to it. */
async function expectUnclipped(page: Page, popup: Locator, items?: Locator) {
  await expect(popup).toBeVisible();
  const vp = page.viewportSize()!;
  const box = (await popup.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(vp.width + 0.5);
  expect(box.y + box.height).toBeLessThanOrEqual(vp.height + 0.5);
  const targets = items ? await items.all() : [popup];
  expect(targets.length).toBeGreaterThan(0);
  for (const it of targets) {
    const b = await it.boundingBox();
    if (!b || b.height === 0) continue;
    const hit = await page.evaluate(({ x, y }) => {
      const el = document.elementFromPoint(x, y);
      return el ? !!el.closest('[role=menu], [role=dialog], [data-testid=filter-menu], [data-testid=columns-menu]') : false;
    }, { x: b.x + b.width / 2, y: b.y + b.height / 2 });
    expect(hit, `item at ${Math.round(b.x)},${Math.round(b.y)} is visible`).toBe(true);
  }
}

// ---- 1. menus -----------------------------------------------------------------------------

for (const vp of [{ width: 1440, height: 900 }, { width: 2000, height: 836 }]) {
  test.describe(`menus @ ${vp.width}x${vp.height}`, () => {
    test.use({ viewport: vp });

    test('download, send, columns, filter, library and account menus are never clipped', async ({ page }) => {
      await openHound(page);
      // the download menu at the right edge of the details pane
      await page.getByRole('button', { name: 'Download as' }).click();
      const dl = page.getByTestId('download-menu');
      await expectUnclipped(page, dl, dl.getByRole('menuitem'));
      await expect(dl.getByRole('menuitem').first()).toContainText(/original/i);
      if (vp.width === 1440) await page.screenshot({ path: `${SHOTS}/live-download-menu.png` });
      await page.keyboard.press('Escape');

      await page.getByRole('button', { name: 'Other device' }).click();
      await expectUnclipped(page, page.locator('.dev-menu'), page.locator('.dev-menu [role=menuitem]'));
      await page.keyboard.press('Escape');

      await page.getByRole('button', { name: 'Columns' }).click();
      await expectUnclipped(page, page.getByTestId('columns-menu'), page.getByTestId('columns-menu').locator('label'));
      await page.keyboard.press('Escape');

      await page.locator('.filter-chooser > button').click();
      await expectUnclipped(page, page.getByTestId('filter-menu'));
      await page.keyboard.press('Escape');

      await page.locator('.lib-switch > button').click();
      await expectUnclipped(page, page.locator('.lib-switch [role=menu]'), page.locator('.lib-switch [role=menuitem]'));
      await page.keyboard.press('Escape');

      await page.getByRole('button', { name: 'Account' }).click();
      await expectUnclipped(page, page.locator('.account [role=menu]'), page.locator('.account .menu-item'));
      await page.keyboard.press('Escape');
    });

    test('the download menu stays inside a narrow details pane and flips up near the bottom', async ({ page }) => {
      await openHound(page);
      // the narrowest details pane
      const sep = page.getByRole('separator', { name: 'Resize the details pane' });
      await sep.focus();
      for (let i = 0; i < 20; i++) await page.keyboard.press('Shift+ArrowRight');
      await page.getByRole('button', { name: 'Download as' }).click();
      await expectUnclipped(page, page.getByTestId('download-menu'), page.getByTestId('download-menu').getByRole('menuitem'));
      await page.keyboard.press('Escape');
      // a short window: the menu still fits (above the button, or capped and scrolling)
      await page.setViewportSize({ width: vp.width, height: 330 });
      await page.getByRole('button', { name: 'Download as' }).click();
      await expectUnclipped(page, page.getByTestId('download-menu'));
    });
  });
}

test('co-authors popover is not clipped', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openAuthor(page, 'Asimov Isaac');
  await page.locator('.scope-header').getByRole('button', { name: /and \d+ more/ }).click();
  await expectUnclipped(page, page.getByRole('dialog', { name: 'Co-authors' }));
});

test.describe('menus on phones', () => {
  test.use({ ...iphone });

  test('download menu on the book page, selection and filter menus', async ({ page }) => {
    await openAuthor(page);
    await page.locator('.m-row', { hasText: 'The Hound of the Baskervilles' }).click();
    await expect(page).toHaveURL(/\/book\//);
    await page.getByRole('button', { name: 'Download as' }).click();
    const dl = page.getByTestId('download-menu');
    await expectUnclipped(page, dl, dl.getByRole('menuitem'));
    await page.screenshot({ path: `${SHOTS}/live-download-menu-phone.png` });
    await page.keyboard.press('Escape');
    await page.goBack();
    await page.locator('.m-row input[type=checkbox]').first().check();
    await page.getByRole('button', { name: 'More' }).click();
    const more = page.getByRole('menu', { name: 'More' });
    await expectUnclipped(page, more, more.getByRole('menuitem'));
    await page.keyboard.press('Escape');
    await page.locator('.filter-chooser > button').click();
    await expectUnclipped(page, page.getByTestId('filter-menu'));
  });
});

// ---- 2. dialog footers on phones -------------------------------------------------------------

for (const vp of [{ width: 390, height: 844 }, { width: 360, height: 740 }]) {
  test(`dialog footers fit a ${vp.width}px screen`, async ({ page }) => {
    await page.setViewportSize(vp);
    await openAuthor(page);
    await page.locator('.m-row input[type=checkbox]').first().check();
    await page.getByRole('button', { name: /…$/ }).first().click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    const footer = dialog.locator('.footer');
    const buttons = footer.getByRole('button');
    for (const b of await buttons.all()) {
      const box = (await b.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(vp.width);
    }
    // the main action has a full-width row of its own
    const primary = footer.locator('button.primary');
    const fb = (await footer.boundingBox())!;
    const pb = (await primary.boundingBox())!;
    expect(pb.width).toBeGreaterThan(fb.width - 40);
    if (vp.width === 390) await page.screenshot({ path: `${SHOTS}/live-send-dialog-phone.png` });
    await page.keyboard.press('Escape');
    // the shelf dialog
    await page.getByRole('button', { name: 'More' }).click();
    await page.getByRole('menuitem', { name: /shelf/i }).click();
    const shelf = page.getByRole('dialog');
    await expect(shelf).toBeVisible();
    for (const b of await shelf.getByRole('button').all()) {
      const box = await b.boundingBox();
      if (box) expect(box.x + box.width).toBeLessThanOrEqual(vp.width);
    }
  });
}

test('send to my phone dialog fits a phone screen', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 });
  // a phone-sized window of a desktop browser (not iOS): the phone button is there
  await openAuthor(page);
  await page.locator('.m-row', { hasText: 'The Hound of the Baskervilles' }).click();
  await page.getByTestId('send-to-phone').click();
  const d = page.getByRole('dialog', { name: 'Send to my phone' });
  await expect(d).toBeVisible();
  for (const b of await d.getByRole('button').all()) {
    const box = await b.boundingBox();
    if (box) expect(box.x + box.width).toBeLessThanOrEqual(360);
  }
});

// ---- 3. follow toast ---------------------------------------------------------------------------

test('Follow shows the message of the new state', async ({ page }) => {
  await openAuthor(page, 'Азимов Айзек');
  const btn = page.getByTestId('follow-author');
  await expect(btn).toBeEnabled();
  if ((await btn.getAttribute('aria-pressed')) === 'true') {
    await btn.click();
    await expect(btn).toHaveAttribute('aria-pressed', 'false');
  }
  await btn.click();
  await expect(btn).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.toast').last()).toHaveText('New books will show up on your start page');
  await btn.click();
  await expect(btn).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('.toast').last()).toHaveText('Stopped following');
});

// ---- 4. bad ids -----------------------------------------------------------------------------------

test('bad ids show a not-found state with a way back', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/l/1/read/999999999');
  const card = page.getByTestId('reader-not-found');
  await expect(card).toContainText('Book not found');
  await expect(card.getByRole('link', { name: 'To the start page' })).toBeVisible();
  await page.goto('/l/1/book/999999999');
  await expect(page.getByTestId('book-not-found')).toContainText('Book not found');
  await page.goto('/l/1/authors/999999999');
  await expect(page.getByTestId('not-found')).toContainText('Author not found');
  await page.getByTestId('not-found').getByRole('link', { name: 'To the list' }).click();
  await expect(page).toHaveURL(/\/l\/1\/authors$/);
  await page.goto('/l/1/series/999999999');
  await expect(page.getByTestId('not-found')).toContainText('Series not found');
  await page.goto('/l/1/genres/999999999');
  await expect(page.getByTestId('not-found')).toContainText('Genre not found');
  await page.goto('/l/1/shelves/999999999');
  await expect(page.getByTestId('not-found')).toContainText('Shelf not found');
  expect(errors).toEqual([]);
});

test('bad ids on a phone: the not-found card is shown', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/l/1/authors/999999999');
  await expect(page.getByTestId('not-found')).toBeVisible();
  await page.goto('/l/1/read/999999999');
  await expect(page.getByTestId('reader-not-found')).toBeVisible();
});

// ---- 10. columns ---------------------------------------------------------------------------------

test('default columns fit the books pane; a rating sort shows its column', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/l/1/genres/2');
  await expect(page.locator('.scroll .brow').first()).toBeVisible({ timeout: 15000 });
  await page.locator('.scroll .brow .title-btn').first().click();
  const vlist = page.locator('.scroll .vlist');
  // no sideways scrolling with the defaults, the title gets the rest
  await expect.poll(() => vlist.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(0);
  const title = page.locator('.scroll .brow .title-cell').first();
  expect((await title.boundingBox())!.width).toBeGreaterThanOrEqual(219);
  // sort by the library rating: its column is on screen, right after the title
  await page.getByTestId('books-sort').selectOption('libRating');
  const head = page.locator('.brow.head');
  await expect(head).toContainText('Library');
  const pane = (await page.locator('.table-wrap').boundingBox())!;
  const col = (await head.locator('.hcell', { hasText: 'Library' }).boundingBox())!;
  expect(col.x + col.width).toBeLessThanOrEqual(pane.x + pane.width);
  await page.getByRole('button', { name: 'Columns' }).click();
  await expect(page.getByTestId('columns-menu').getByLabel('Library rating')).toBeDisabled();
  await page.keyboard.press('Escape');
  await page.getByTestId('books-sort').selectOption('date');
  await expect(head).not.toContainText('Library');
});

test('a narrow books pane drops default columns; ticking one brings it back', async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 900 });
  await openHound(page);
  const head = page.locator('.brow.head');
  // 1200 − nav − list − details leaves too little for Size and Added beside the title
  await expect(head).not.toContainText('Size');
  await page.getByRole('button', { name: 'Columns' }).click();
  const menu = page.getByTestId('columns-menu');
  await expect(menu.getByLabel('Size')).not.toBeChecked();
  await expect(menu).toContainText('no room');
  await menu.getByLabel('Size').check();
  await expect(head).toContainText('Size');
  await menu.getByLabel('Size').uncheck();
  await page.keyboard.press('Escape');
  await page.evaluate(() => fetch('/api/v1/me/prefs', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: '{}' }));
});

// ---- 12. search filters in the URL -----------------------------------------------------------

test('search filters and sort live in the URL: back/forward and shared links keep them', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/l/1/search?q=hound&exact=1');
  await expect(page.getByText(/results? for/)).toBeVisible({ timeout: 15000 });
  const box = page.locator('.facet .fl input[type="checkbox"]').first();
  await box.check();
  await expect(page).toHaveURL(/[?&]genre=\d+/);
  await expect(page).toHaveURL(/exact=1/);
  await expect(page).toHaveURL(/q=hound/);
  await page.getByTestId('search-sort').selectOption('libRating');
  await expect(page).toHaveURL(/sort=libRating/);
  const shared = page.url();

  // back: the sort goes, the genre stays; back again: no filter
  await page.goBack();
  await expect(page).not.toHaveURL(/sort=/);
  await expect(page.locator('.facet .fl input[type="checkbox"]').first()).toBeChecked();
  await page.goBack();
  await expect(page).not.toHaveURL(/genre=/);
  await expect(page.locator('.summary .chip')).toHaveCount(0);
  await page.goForward();
  await expect(page.locator('.summary .chip')).toHaveCount(1);

  // leave and come back
  await page.getByRole('link', { name: 'Authors' }).click();
  await page.goBack();
  await expect(page).toHaveURL(/genre=/);
  await expect(page.locator('.summary .chip')).toHaveCount(1);

  // a shared link in a fresh page
  const req = page.waitForRequest((r) => r.url().includes('/search?') && r.url().includes('genre=') && r.url().includes('sort=lib'));
  await page.goto(shared);
  await req;
  await expect(page.getByTestId('search-sort')).toHaveValue('libRating');
  await expect(page.locator('.summary .chip')).toHaveCount(1);
  await page.evaluate(() => fetch('/api/v1/me/prefs', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: '{}' }));
});

// ---- 13. cover grid --------------------------------------------------------------------------

test('cover grid: a click shows the details (reopening a folded pane), the checkbox selects', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openHound(page);
  await page.getByRole('button', { name: 'Hide details' }).click();
  await page.getByRole('button', { name: 'Cover view' }).click();
  const card = page.getByTestId('cover-card').filter({ hasText: 'A Study in Scarlet' });
  await card.locator('.card-open').click();
  const details = page.getByRole('complementary', { name: 'Details' });
  await expect(details.locator('.book-title')).toHaveText('A Study in Scarlet');
  await expect(card).toHaveClass(/selected/);
  // the checkbox ticks without changing the current book
  const other = page.getByTestId('cover-card').filter({ hasText: 'The Sign of the Four' });
  await other.getByRole('checkbox').check();
  await expect(other).toHaveClass(/checked/);
  await expect(details.locator('.book-title')).toHaveText('A Study in Scarlet');
  await other.getByRole('checkbox').uncheck();
  // clicking the cover again: same result every time
  await other.locator('.card-open').click();
  await expect(details.locator('.book-title')).toHaveText('The Sign of the Four');
  await page.getByRole('button', { name: 'Table view' }).click();
  await page.evaluate(() => fetch('/api/v1/me/prefs', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: '{}' }));
});

// ---- 14. annotation URLs -----------------------------------------------------------------------

test('long annotation URLs wrap and become safe links', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const long = 'https://example.org/a/very/long/path/' + 'segment'.repeat(30) + '?q=1&x=2';
  await page.route(/\/api\/v1\/libraries\/1\/books\/\d+$/, async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    body.annotation = `<p>Read more at ${long}. Also &lt;img src=x onerror=alert(1)&gt; and javascript:alert(1) (see http://example.com/x_(y)).</p>`;
    await route.fulfill({ response: res, json: body });
  });
  await openHound(page);
  const ann = page.getByTestId('annotation');
  await expect(ann).toBeVisible();
  const links = ann.getByRole('link');
  await expect(links).toHaveCount(2);
  await expect(links.first()).toHaveAttribute('href', long);
  await expect(links.first()).toHaveAttribute('rel', 'noopener noreferrer nofollow');
  await expect(links.first()).toHaveAttribute('target', '_blank');
  await expect(links.nth(1)).toHaveAttribute('href', 'http://example.com/x_(y)');
  // the escaped markup stays text
  await expect(ann.locator('img')).toHaveCount(0);
  await expect(ann).toContainText('<img src=x onerror=alert(1)>');
  // nothing runs past the pane
  const details = page.getByRole('complementary', { name: 'Details' });
  expect(await details.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(0);
  const a = (await ann.boundingBox())!;
  const d = (await details.boundingBox())!;
  expect(a.x + a.width).toBeLessThanOrEqual(d.x + d.width);
});

// ---- 15. last error --------------------------------------------------------------------------

test('Open Library "Last error" wraps', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const err = 'GET https://openlibrary.org/search.json?q=' + 'x'.repeat(260) + ': HTTP 503 Service Unavailable';
  await page.route('**/api/v1/settings', async (route) => {
    if (route.request().method() !== 'GET') return route.continue();
    const res = await route.fetch();
    const body = await res.json();
    body.externalRatings = { ...body.externalRatings, lastError: err };
    await route.fulfill({ response: res, json: body });
  });
  await page.goto('/settings/server');
  const el = page.getByTestId('ext-last-error');
  await expect(el).toBeVisible();
  const box = (await el.boundingBox())!;
  expect(box.x + box.width).toBeLessThanOrEqual(1440);
  expect(await el.evaluate((e) => e.scrollWidth - e.clientWidth)).toBeLessThanOrEqual(0);
  expect(box.height).toBeGreaterThan(30); // wrapped over several lines
  expect(await page.locator('.settings-page').evaluate((e) => e.scrollWidth - e.clientWidth)).toBeLessThanOrEqual(0);
});

// ---- 16/17. reader --------------------------------------------------------------------------

test('reader keeps its place when the contents open and close, and cleans up when left', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push(m.text()); });
  await page.setViewportSize({ width: 1440, height: 900 });
  await openHound(page);
  await page.getByRole('button', { name: 'Read' }).click();
  const reader = page.locator('.reader');
  await expect(reader).toHaveAttribute('data-ready', 'true', { timeout: 15000 });
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(300);
  await page.keyboard.press('ArrowRight');
  await page.waitForTimeout(500);
  const before = await reader.getAttribute('data-cfi');
  expect(before).toBeTruthy();
  await page.getByRole('button', { name: 'Contents' }).click();
  await expect(page.getByRole('complementary', { name: 'Contents' })).toBeVisible();
  await page.waitForTimeout(700);
  await page.getByRole('button', { name: 'Contents' }).click();
  await expect(page.getByRole('complementary', { name: 'Contents' })).toBeHidden();
  await expect.poll(() => reader.getAttribute('data-cfi'), { timeout: 3000 }).toBe(before);
  // a window resize keeps it too
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.waitForTimeout(700);
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect.poll(() => reader.getAttribute('data-cfi'), { timeout: 3000 }).toBe(before);

  // leave the reader, then resize: no errors from a torn-down paginator
  await page.getByRole('button', { name: 'Back' }).first().click();
  await expect(page).not.toHaveURL(/\/read\//);
  await page.setViewportSize({ width: 1000, height: 700 });
  await page.waitForTimeout(500);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.waitForTimeout(500);
  expect(errors.filter((e) => !/favicon/.test(e))).toEqual([]);
});

// ---- 18. keyboard -------------------------------------------------------------------------------

test('book list: arrows move over rows, group headers and editions; Home/End, Space, Enter, ←/→', async ({ page }) => {
  await page.setViewportSize({ width: 1600, height: 900 });
  await openAuthor(page, 'Азимов Айзек');
  const grid = page.getByTestId('books-grid');
  await grid.focus();
  const current = async () => page.evaluate(() => {
    const g = document.querySelector('[data-testid=books-grid]')!;
    const id = g.getAttribute('aria-activedescendant');
    const el = id ? document.getElementById(id) : null;
    return el ? { kind: el.dataset.testid ?? '', text: el.textContent?.trim().slice(0, 60) ?? '' } : null;
  });
  await page.keyboard.press('Home');
  expect((await current())!.kind).toBe('group-row');
  await page.keyboard.press('ArrowDown');
  const first = (await current())!;
  expect(first.kind).toBe('book-row');
  // the current book is shown in the details pane
  const title = page.getByRole('complementary', { name: 'Details' }).locator('.book-title');
  await expect(title).toBeVisible();
  expect(first.text).toContain((await title.textContent())!.trim());
  // Space ticks it
  await page.keyboard.press(' ');
  await expect(page.locator('.scroll .brow.cursor input[type=checkbox]')).toBeChecked();
  await page.keyboard.press(' ');
  await expect(page.locator('.scroll .brow.cursor input[type=checkbox]')).not.toBeChecked();
  // ← folds the group and moves to its header, → unfolds it
  await page.keyboard.press('ArrowLeft');
  expect((await current())!.kind).toBe('group-row');
  await expect(page.locator('.group-head.cursor')).toHaveAttribute('aria-expanded', 'false');
  await page.keyboard.press('ArrowRight');
  await expect(page.locator('.group-head.cursor')).toHaveAttribute('aria-expanded', 'true');
  // End: the last row, scrolled into view
  await page.keyboard.press('End');
  const last = page.locator('.scroll .cursor');
  await expect(last).toBeInViewport();
  // editions: → opens a work's editions, ↓ walks into them, ← goes back up
  const work = page.locator('.scroll .brow', { has: page.getByTestId('editions-toggle') }).first();
  await work.click();
  await page.keyboard.press('ArrowRight');
  await expect(work.getByTestId('editions-toggle')).toHaveAttribute('aria-expanded', 'true');
  // the editions load on demand
  await expect(page.getByTestId('edition-row').first()).toBeVisible();
  await page.keyboard.press('ArrowDown');
  expect((await current())!.kind).toBe('edition-row');
  await page.keyboard.press('ArrowLeft');
  expect((await current())!.kind).toBe('book-row');
  await expect(work.getByTestId('editions-toggle')).toHaveAttribute('aria-expanded', 'false');
  // Enter reopens a folded details pane
  await page.getByRole('button', { name: 'Hide details' }).click();
  await grid.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Hide details' })).toBeVisible();
});

test('author list and genre tree are arrow-navigable', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill('Doyle');
  const rows = page.locator('.browser .arow');
  await expect(rows.first()).toBeVisible();
  await rows.first().focus();
  await page.keyboard.press('ArrowDown');
  await expect(rows.nth(1)).toBeFocused();
  await page.keyboard.press('End');
  await expect(page.locator('.browser .arow:focus')).toBeVisible();
  await page.keyboard.press('Home');
  await expect(rows.first()).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/\/authors\/\d+/);

  await page.goto('/l/1/genres');
  const parents = page.locator('.tree a.row.parent');
  await parents.first().focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.locator('.tree .exp').first()).toHaveAttribute('aria-expanded', 'true');
  await page.keyboard.press('ArrowDown');
  await expect(page.locator('.tree a.row.child').first()).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(parents.first()).toBeFocused();
  await page.keyboard.press('ArrowLeft');
  await expect(page.locator('.tree .exp').first()).toHaveAttribute('aria-expanded', 'false');
});

// ---- 19. focus return ---------------------------------------------------------------------------

test('dialogs keep focus inside and return it to the opener', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await openHound(page);
  const phoneBtn = page.getByTestId('send-to-phone');
  await phoneBtn.click();
  const d = page.getByRole('dialog', { name: 'Send to my phone' });
  await expect(d).toBeVisible();
  for (let i = 0; i < 8; i++) {
    await page.keyboard.press('Tab');
    expect(await page.evaluate(() => !!document.activeElement?.closest('[role=dialog]'))).toBe(true);
  }
  await page.keyboard.press('Escape');
  await expect(d).toBeHidden();
  await expect(phoneBtn).toBeFocused();

  // a dialog opened from a menu item returns focus to the menu's button
  const other = page.getByRole('button', { name: 'Other device' });
  await other.click();
  await page.locator('.dev-menu [role=menuitem]').first().click();
  const send = page.getByRole('dialog');
  await expect(send).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(send).toBeHidden();
  await expect(other).toBeFocused();

  // closing with the Close button
  await phoneBtn.click();
  await d.getByRole('button', { name: 'Close' }).last().click();
  await expect(phoneBtn).toBeFocused();
});

// ---- 20. letter strip on phones --------------------------------------------------------------

test.describe('phone letter strip', () => {
  test.use({ ...iphone });

  test('wide touch targets, a scrub bubble, counts clear of the scrollbar', async ({ page }) => {
    await page.goto('/l/1/authors');
    const strip = page.getByTestId('letter-strip');
    await expect(strip).toBeVisible({ timeout: 15000 });
    const btn = strip.locator('button:not([disabled])').first();
    const bb = (await btn.boundingBox())!;
    expect(bb.width).toBeGreaterThanOrEqual(32);
    // scrub: press and slide along the strip; the letter under the finger shows in a bubble
    const sb = (await strip.boundingBox())!;
    const x = sb.x + sb.width / 2;
    await strip.dispatchEvent('pointerdown', { pointerType: 'touch', pointerId: 7, clientX: x, clientY: sb.y + 10, isPrimary: true, bubbles: true });
    await expect(page.getByTestId('letter-bubble')).toBeVisible();
    const first = await page.getByTestId('letter-bubble').textContent();
    await strip.dispatchEvent('pointermove', { pointerType: 'touch', pointerId: 7, clientX: x, clientY: sb.y + sb.height * 0.6, isPrimary: true, bubbles: true });
    await expect(page.getByTestId('letter-bubble')).not.toHaveText(first!);
    const letter = (await page.getByTestId('letter-bubble').textContent())!;
    await page.screenshot({ path: `${SHOTS}/live-letter-scrub-phone.png` });
    await strip.dispatchEvent('pointerup', { pointerType: 'touch', pointerId: 7, clientX: x, clientY: sb.y + sb.height * 0.6, isPrimary: true, bubbles: true });
    await expect(page.getByTestId('letter-bubble')).toBeHidden();
    await expect(strip.locator('button.active')).toHaveText(letter);
    // the counts end well left of the list's right edge (overlay scrollbars)
    const row = page.locator('.browser .arow').first();
    const count = (await row.locator('.count').boundingBox())!;
    const list = (await page.locator('.browser .vlist').boundingBox())!;
    expect(list.x + list.width - (count.x + count.width)).toBeGreaterThanOrEqual(12);
  });
});

// ---- 21. folder devices: an empty folder is the export root -------------------------------

test('a folder device without a folder is labelled as the export root, and can be used', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.route('**/api/v1/devices', async (route) => {
    if (route.request().method() !== 'GET') return route.continue();
    const res = await route.fetch();
    const list = await res.json();
    for (const d of list) if (d.kind === 'folder') d.target = null;
    await route.fulfill({ response: res, json: list });
  });
  await openHound(page);
  await page.getByRole('checkbox', { name: 'Select The Hound of the Baskervilles' }).check();
  await page.getByTestId('selection-send').click();
  const dialog = page.getByRole('dialog');
  const card = dialog.getByRole('button', { name: /Server folder/ });
  await expect(card).toBeEnabled();
  await expect(card.getByTestId('device-folder')).toHaveText('Export folder (root)');
  await card.click();
  await expect(dialog.getByPlaceholder('Export folder (root)')).toHaveValue('');
  await expect(dialog.getByTestId('send-submit')).toBeEnabled();
  await page.keyboard.press('Escape');
  await page.getByRole('checkbox', { name: 'Select The Hound of the Baskervilles' }).uncheck();

  // Settings → Devices: the same label, not a warning
  await page.goto('/settings/devices');
  const row = page.locator('.device-row', { hasText: 'Server folder' });
  await expect(row.getByTestId('device-folder')).toHaveText('Export folder (root)');
});

test('a folder device with a sub-folder shows it', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/settings/devices');
  // the mock's Server folder has the sub-folder "incoming"
  await expect(page.locator('.device-row', { hasText: 'Server folder' })).toContainText('incoming');
});
