import { test, expect, devices, type Page, type Route } from '@playwright/test';

// Second round of fixes from testing the live deployment: user rows in Settings, the book
// table beside the details pane (also with a column list saved by an older version), the 404
// page, Expand / Collapse all, the Cyrillic letter strip, the top search box, the phone
// Search tab, the sidebar highlight, typing while a page loads, clicks while a page loads and
// the reader's default theme.

const { defaultBrowserType: _b, ...iphone } = devices['iPhone 13'];
const SHOTS = '../docs/web/screenshots';

const resetPrefs = (page: Page) =>
  page.evaluate(() => fetch('/api/v1/me/prefs', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: '{}' }));

async function openAuthor(page: Page, name = 'Doyle Arthur Conan') {
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill(name);
  await page.getByRole('button', { name: new RegExp(name) }).first().click();
  await expect(page.locator('.scroll [role=row], .m-row, .m-group').first()).toBeVisible({ timeout: 15000 });
}

// ---- 1. users ------------------------------------------------------------------------------

const USERS = [
  { id: 1, username: 'admin', role: 'admin', email: null, hasPassword: true, sso: null },
  { id: 2, username: 'kwull (2)', role: 'reader', email: 'kwull@kwull.com', hasPassword: false, sso: { email: 'kwull@kwull.com' } },
  {
    id: 3, username: 'a-very-long-username-that-goes-on-and-on@example.org', role: 'reader',
    email: 'someone.with.a.long.address@example.org', hasPassword: true, sso: { email: 'someone.with.a.long.address@example.org' },
  },
];

for (const vp of [{ width: 1302, height: 800 }, { width: 1440, height: 900 }, { width: 390, height: 844 }]) {
  test(`user rows keep every control inside the card @ ${vp.width}`, async ({ page }) => {
    await page.setViewportSize(vp);
    await page.route('**/api/v1/users', (r) => (r.request().method() === 'GET' ? r.fulfill({ json: USERS }) : r.fallback()));
    await page.goto('/settings/users');
    const rows = page.getByTestId('user-row');
    await expect(rows).toHaveCount(3);
    for (let i = 0; i < 3; i++) {
      const row = rows.nth(i);
      const box = (await row.boundingBox())!;
      for (const el of await row.locator('input, select, button, .badge, .uname').all()) {
        const b = (await el.boundingBox())!;
        expect(b.x, await el.evaluate((e) => e.outerHTML.slice(0, 60))).toBeGreaterThanOrEqual(box.x);
        expect(b.x + b.width).toBeLessThanOrEqual(box.x + box.width + 0.5);
      }
      // the e-mail field is a usable field, not a sliver
      expect((await row.locator('input.email').boundingBox())!.width).toBeGreaterThanOrEqual(180);
    }
    // nothing sticks out of the page sideways
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    const panel = (await page.locator('.panel').boundingBox())!;
    const add = (await page.getByRole('button', { name: 'Add user' }).boundingBox())!;
    expect(add.x + add.width).toBeLessThanOrEqual(panel.x + panel.width + 0.5);
    if (vp.width === 1302) await page.screenshot({ path: `${SHOTS}/retest-users-1302.png` });
  });
}

// ---- 2. the book table beside the details pane ---------------------------------------------------

async function openNewWithDetails(page: Page) {
  await page.goto('/l/1/new');
  await expect(page.locator('.scroll .brow .title-btn').first()).toBeVisible({ timeout: 15000 });
  await page.locator('.scroll .brow .title-btn').first().click();
  await expect(page.getByRole('complementary', { name: 'Details' })).toBeVisible();
}
const overflow = (page: Page) => page.locator('.scroll .vlist').evaluate((el) => el.scrollWidth - el.clientWidth);
const headText = (page: Page) => page.locator('.brow.head').innerText();

test.describe('book table @ 1302x800', () => {
  test.use({ viewport: { width: 1302, height: 800 } });

  test('default columns fit with the details pane open and come back when it closes', async ({ page }) => {
    await openNewWithDetails(page);
    await expect.poll(() => overflow(page)).toBeLessThanOrEqual(0);
    expect((await page.locator('.scroll .brow .title-cell').first().boundingBox())!.width).toBeGreaterThanOrEqual(219);
    const full = await headText(page);
    expect(full).toContain('Author');
    // the pane is live: a narrower window drops columns, closing the details brings them back
    await page.setViewportSize({ width: 1000, height: 800 });
    await expect.poll(async () => (await headText(page)).length).toBeLessThan(full.length);
    await expect.poll(() => overflow(page)).toBeLessThanOrEqual(0);
    await page.getByRole('button', { name: 'Hide details' }).click();
    await expect.poll(() => headText(page)).toBe(full);
    await expect.poll(() => overflow(page)).toBeLessThanOrEqual(0);
    await page.getByRole('button', { name: 'Show details' }).click();
    await expect.poll(async () => (await headText(page)).length).toBeLessThan(full.length);
    // … and a wider window again
    await page.setViewportSize({ width: 1302, height: 800 });
    await expect.poll(() => headText(page)).toBe(full);
  });

  test('a column list saved by an older version fits too, and the menu says what is hidden', async ({ page }) => {
    // an explicit list from before the columns gave way (plain keys, as the old menu saved them)
    await page.goto('/l/1/new');
    await page.evaluate(() => fetch('/api/v1/me/prefs', {
      method: 'PUT', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ 'cols.other': ['author', 'series', 'genre', 'language', 'format', 'size', 'added'] }),
    }));
    await page.evaluate(() => localStorage.removeItem('freelib.prefs'));
    await openNewWithDetails(page);
    await expect.poll(() => overflow(page)).toBeLessThanOrEqual(0);
    expect((await page.locator('.scroll .brow .title-cell').first().boundingBox())!.width).toBeGreaterThanOrEqual(219);
    const head = await headText(page);
    expect(head).toContain('Author');
    expect(head).not.toContain('Language');
    await page.getByRole('button', { name: 'Columns' }).click();
    const menu = page.getByTestId('columns-menu');
    await expect(menu.getByTestId('columns-hidden')).toHaveText(/^\d+ columns hidden: not enough room$/);
    await expect(menu.getByLabel('Language')).not.toBeChecked();
    await page.screenshot({ path: `${SHOTS}/retest-table-details.png` });
    // asking for a hidden column keeps it (the table then scrolls sideways)
    await menu.getByLabel('Language').check();
    await expect(page.locator('.brow.head')).toContainText('Language');
    await expect(menu.getByLabel('Language')).toBeChecked();
    await page.keyboard.press('Escape');
    // wider pane: everything shows, nothing hidden
    await page.getByRole('button', { name: 'Hide details' }).click();
    await page.setViewportSize({ width: 2000, height: 800 });
    await page.getByRole('button', { name: 'Columns' }).click();
    await expect(page.getByTestId('columns-hidden')).toHaveCount(0);
    await expect(page.locator('.brow.head')).toContainText('Genre');
    await page.keyboard.press('Escape');
    await page.getByRole('button', { name: 'Show details' }).click();
    await resetPrefs(page);
  });
});

// ---- 3. 404 ---------------------------------------------------------------------------------

test('an unknown address shows a not-found page with the path and ways back', async ({ page }) => {
  await page.setViewportSize({ width: 1302, height: 800 });
  await page.goto('/l/1/home');
  await expect(page.getByRole('link', { name: 'New arrivals' }).first()).toBeVisible({ timeout: 15000 });
  await page.goto('/l/1/no-such-page?x=1');
  const card = page.getByTestId('page-not-found');
  await expect(card).toContainText('Page not found');
  await expect(page.getByTestId('not-found-path')).toHaveText('/l/1/no-such-page?x=1');
  // inside the shell: the sidebar is there, nothing in it is highlighted
  await expect(page.locator('.sidenav [aria-current=page]')).toHaveCount(0);
  await page.screenshot({ path: `${SHOTS}/retest-404.png` });
  await card.getByRole('button', { name: 'Back' }).click();
  await expect(page).toHaveURL(/\/l\/1\/home$/);
  await page.goto('/whatever/else');
  await page.getByTestId('page-not-found').getByRole('link', { name: 'To the start page' }).click();
  await expect(page).toHaveURL(/\/l\/1\/home$/);
});

test('the not-found page works signed out and on a phone', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route('**/api/v1/session', (r) => r.fulfill({ json: { user: null, openMode: false, auth: { password: true, oidc: null } } }));
  await page.goto('/no/such/page');
  const card = page.getByTestId('page-not-found');
  await expect(card).toContainText('Page not found');
  await expect(page.getByTestId('not-found-path')).toHaveText('/no/such/page');
  const link = card.getByRole('link', { name: 'To the start page' });
  await expect(link).toHaveAttribute('href', '/');
  const b = (await card.locator('.card').boundingBox())!;
  expect(b.x).toBeGreaterThanOrEqual(0);
  expect(b.x + b.width).toBeLessThanOrEqual(390);
  await link.click();
  await expect(page.getByLabel('Username')).toBeVisible();
});

// ---- 4. Expand all / Collapse all -------------------------------------------------------------

test('Expand all turns into Collapse all only when every group is open', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  // ~20 series: the groups start folded
  await openAuthor(page, 'Asimov Isaac');
  const btn = page.getByTestId('expand-all');
  await expect(btn).toHaveText('Expand all');
  const toggles = page.locator('.scroll .group-head .gtoggle');
  await toggles.nth(0).click();
  await expect(toggles.nth(0)).toHaveAttribute('aria-expanded', 'true');
  await expect(btn).toHaveText('Expand all');
  await toggles.nth(1).click();
  await expect(btn).toHaveText('Expand all');
  await btn.click();
  await expect(btn).toHaveText('Collapse all');
  // one group folded again: back to Expand all, which opens it
  await page.locator('.scroll .group-head .gtoggle').first().click();
  await expect(page.locator('.scroll .group-head .gtoggle').first()).toHaveAttribute('aria-expanded', 'false');
  await expect(btn).toHaveText('Expand all');
  await btn.click();
  await expect(page.locator('.scroll .group-head .gtoggle[aria-expanded=false]')).toHaveCount(0);
  await expect(btn).toHaveText('Collapse all');
  await btn.click();
  await expect(page.locator('.scroll .brow')).toHaveCount(0);
  await expect(btn).toHaveText('Expand all');
});

// ---- 5. the Cyrillic letter strip ------------------------------------------------------------------

test('the letter strip follows the list order: Ы between Щ and Э, Ukrainian letters after Я', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  // a list in the server's order (lower-cased code points): … щ ы э я є і ї ґ
  const names = ['Абрамов', 'Борисов', 'Щербаков', 'Ыбыков', 'Эренбург', 'Яковлев', 'Єрмоленко', 'Іваненко', 'Їжакевич', 'Ґалаґан'];
  const rows = names.map((n, i) => [1000 + i, n, 1]);
  const letters = names.map((n, i) => [n[0], 1, i]);
  await page.route(/\/api\/v1\/libraries\/1\/authors(\?.*)?$/, (r) => r.fulfill({ json: { version: 1, columns: ['id', 'name', 'count'], rows, letters } }));
  await page.goto('/l/1/authors');
  const strip = page.getByLabel('Jump to letter');
  await expect(strip.getByRole('button', { name: /^Ы: 1$/ })).toBeVisible({ timeout: 15000 });
  const order = (await strip.getByRole('button').allTextContents()).join('');
  expect(order).toBe('АБВГДЕЖЗИЙКЛМНОПРСТУФХЦЧШЩЫЭЮЯЄІЇҐ');
  // the list itself is in the same order
  const shown = await page.locator('.arow .name').allTextContents();
  expect(shown).toEqual(names);
  await strip.getByRole('button', { name: /^Ы: 1$/ }).click();
  await expect(strip.getByRole('button', { name: /^Ы: 1$/ })).toHaveClass(/active/);
});

// ---- 6. the top search box -------------------------------------------------------------------------

test('the top search box shows the query only on its search page', async ({ page }) => {
  await page.setViewportSize({ width: 1302, height: 800 });
  await page.goto('/l/1/search?q=hound');
  const box = page.locator('.topbar input[role=combobox]');
  await expect(box).toHaveValue('hound', { timeout: 15000 });
  await page.locator('.sidenav a[href="/l/1/new"]').click();
  await expect(page).toHaveURL(/\/l\/1\/new$/);
  await expect(box).toHaveValue('');
  await page.goBack();
  await expect(page).toHaveURL(/search\?q=hound/);
  await expect(box).toHaveValue('hound');
  // a new search from the box, then an author from the results
  await box.fill('doyle');
  await box.press('Enter');
  await expect(page).toHaveURL(/search\?q=doyle/);
  await expect(box).toHaveValue('doyle');
  await page.locator('.series-card').first().click();
  await expect(page).toHaveURL(/\/authors\/\d+/);
  await expect(box).toHaveValue('');
});

// ---- 7. the phone Search tab ------------------------------------------------------------------------

test.describe('phone', () => {
  test.use({ ...iphone });

  test('the Search tab opens the search page with its box focused', async ({ page }) => {
    await page.goto('/l/1/home');
    const tab = page.locator('.phone-nav a', { hasText: 'Search' });
    await expect(tab).toBeVisible({ timeout: 15000 });
    await tab.tap();
    await expect(page).toHaveURL(/\/l\/1\/search$/);
    await expect(page.locator('input[data-search-input]')).toBeFocused();
    await expect(tab).toHaveAttribute('aria-current', 'page');
    // on the search page already: the box takes the focus again
    await page.locator('input[data-search-input]').fill('hound');
    await page.locator('input[data-search-input]').press('Enter');
    await expect(page).toHaveURL(/q=hound/);
    await page.locator('input[data-search-input]').blur();
    await tab.tap();
    await expect(page.locator('input[data-search-input]')).toBeFocused();
    await expect(page).toHaveURL(/q=hound/);
  });
});

// ---- 8. the sidebar highlight ------------------------------------------------------------------------

test('the sidebar highlights the current page only; a search highlights nothing', async ({ page }) => {
  await page.setViewportSize({ width: 1302, height: 800 });
  await page.goto('/l/1/home');
  const newItem = page.locator('.sidenav a[href="/l/1/new"]');
  await newItem.click();
  await expect(newItem).toHaveAttribute('aria-current', 'page');
  const idle = await page.locator('.sidenav a[href="/l/1/series"]').evaluate((e) => getComputedStyle(e).backgroundColor);
  // the pointer stays on "New arrivals" while the search is typed
  await page.keyboard.press('/');
  await page.keyboard.type('hound');
  await page.keyboard.press('Enter');
  await expect(page).toHaveURL(/search\?q=hound/);
  await expect(page.locator('.sidenav [aria-current=page]')).toHaveCount(0);
  await expect(page.locator('.sidenav .nav.active')).toHaveCount(0);
  expect(await newItem.evaluate((e) => e.matches(':hover'))).toBe(true);
  expect(await newItem.evaluate((e) => getComputedStyle(e).backgroundColor)).toBe(idle);
  // moving the pointer brings the hover back
  await page.mouse.move(60, 180);
  await page.mouse.move(60, 134);
  await page.locator('.sidenav a[href="/l/1/authors"]').click();
  await expect(page.locator('.sidenav [aria-current=page]')).toHaveText('Authors');
});

// ---- 9. typing while the page loads ------------------------------------------------------------------

async function delayed(route: Route, ms: number) {
  await new Promise((r) => setTimeout(r, ms));
  await route.fallback();
}

test('typing into the author filter while the page loads is kept', async ({ page }) => {
  await page.setViewportSize({ width: 1302, height: 800 });
  // the library list and the name list are slow
  await page.route('**/api/v1/libraries', (r) => delayed(r, 1500));
  await page.route(/\/api\/v1\/libraries\/1\/authors(\?.*)?$/, (r) => delayed(r, 1500));
  await page.goto('/l/1/authors');
  const early = page.getByTestId('early-filter');
  await early.click();
  await page.keyboard.type('Doyle');
  const real = page.locator('section.browser').getByLabel('Filter authors');
  await expect(real).toBeVisible({ timeout: 15000 });
  await expect(real).toHaveValue('Doyle');
  await expect(real).toBeFocused();
  // still loading the list: typing goes on in the same box
  await page.keyboard.type(' Arthur');
  await expect(real).toHaveValue('Doyle Arthur');
  await expect(page.getByRole('button', { name: /Doyle Arthur Conan/ }).first()).toBeVisible({ timeout: 15000 });
  await expect(real).toHaveValue('Doyle Arthur');
});

test('the author filter is kept when coming back to Authors', async ({ page }) => {
  await page.setViewportSize({ width: 1302, height: 800 });
  await openAuthor(page, 'Doyle Arthur Conan');
  const filter = page.locator('section.browser').getByLabel('Filter authors');
  await expect(filter).toHaveValue('Doyle Arthur Conan');
  await page.locator('.sidenav a[href="/l/1/new"]').click();
  await expect(page).toHaveURL(/\/l\/1\/new$/);
  await page.locator('.sidenav a[href="/l/1/authors"]').click();
  await expect(filter).toHaveValue('Doyle Arthur Conan');
  await expect(page.getByRole('button', { name: /Doyle Arthur Conan/ }).first()).toBeVisible();
  // Series has a filter of its own
  await page.locator('.sidenav a[href="/l/1/series"]').click();
  await expect(page.locator('section.browser').getByLabel('Filter series')).toHaveValue('');
  await page.goBack();
  await expect(filter).toHaveValue('Doyle Arthur Conan');
});

// ---- 10. clicks while the page loads -----------------------------------------------------------------

test('the view buttons work while the author list and the books still load', async ({ page }) => {
  await page.setViewportSize({ width: 1302, height: 800 });
  await openAuthor(page, 'Doyle Arthur Conan');
  const url = page.url();
  await page.evaluate(() => indexedDB.deleteDatabase('freelib-cache'));
  await page.route(/\/api\/v1\/libraries\/1\/authors(\?.*)?$/, (r) => delayed(r, 4000));
  await page.route(/\/api\/v1\/libraries\/1\/books\?/, (r) => delayed(r, 1500));
  await page.goto(url);
  // the books pane and its toolbar do not wait for the whole author list
  const cover = page.getByRole('button', { name: 'Cover view' });
  await expect(cover).toBeVisible({ timeout: 3000 });
  await expect(page.locator('.arow')).toHaveCount(0);
  await cover.click();
  await expect(cover).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.grid-view')).toBeVisible({ timeout: 15000 });
  await page.getByRole('button', { name: 'Table view' }).click();
  await expect(page.locator('.scroll .brow').first()).toBeVisible();
  await expect(page.locator('.arow').first()).toBeVisible({ timeout: 15000 });
  await resetPrefs(page);
});

// ---- 11. the reader's default theme --------------------------------------------------------------------

test('the reader follows the app theme until a theme is picked there', async ({ page }) => {
  await page.setViewportSize({ width: 1302, height: 800 });
  // a dark app, and reader prefs as older versions saved them on every visit
  await page.addInitScript(() => {
    if (sessionStorage.getItem('seeded')) return;
    sessionStorage.setItem('seeded', '1');
    localStorage.setItem('freelib.theme', 'dark');
    localStorage.setItem('freelib.reader.prefs', JSON.stringify({ fontSize: 110, theme: 'light' }));
  });
  await openAuthor(page);
  await page.getByText('The Hound of the Baskervilles').first().click();
  await page.getByRole('button', { name: 'Read' }).click();
  const reader = page.locator('.reader');
  await expect(reader).toHaveAttribute('data-ready', 'true', { timeout: 15000 });
  await expect(reader).toHaveAttribute('data-theme', 'dark');
  await expect(page.locator('.font-ctl .pct')).toHaveText('110%');
  // an explicit choice wins, also over a dark app, and is kept
  await page.getByRole('button', { name: 'Light', exact: true }).click();
  await expect(reader).toHaveAttribute('data-theme', 'light');
  await page.reload();
  await expect(reader).toHaveAttribute('data-theme', 'light', { timeout: 15000 });
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('freelib.reader.prefs')!))).toMatchObject({ theme: 'light', themeChosen: true });
});

test('with no choice, the reader follows a light app and the system theme', async ({ page }) => {
  await page.setViewportSize({ width: 1302, height: 800 });
  await page.emulateMedia({ colorScheme: 'dark' });
  await openAuthor(page);
  await page.getByText('The Hound of the Baskervilles').first().click();
  await page.getByRole('button', { name: 'Read' }).click();
  const reader = page.locator('.reader');
  // app theme "system", the system is dark
  await expect(reader).toHaveAttribute('data-theme', 'dark', { timeout: 15000 });
  await page.emulateMedia({ colorScheme: 'light' });
  await expect(reader).toHaveAttribute('data-theme', 'light');
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('freelib.reader.prefs') ?? '{}').theme)).toBeUndefined();
});
