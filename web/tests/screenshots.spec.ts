import { test, expect } from '@playwright/test';

const DIR = 'test-results/screenshots';

test.describe('screenshots @ 1440x900', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('main (authors + books + details)', async ({ page }) => {
    await page.goto('/l/1/authors');
    await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
    await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
    await page.getByText('The Hound of the Baskervilles').first().click();
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${DIR}/main-1440x900.png` });
  });

  test('search', async ({ page }) => {
    await page.goto('/l/1/search?q=hound');
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${DIR}/search-1440x900.png` });
  });

  test('libraries', async ({ page }) => {
    await page.goto('/libraries');
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${DIR}/libraries-1440x900.png` });
  });

  test('send dialog', async ({ page }) => {
    await page.goto('/l/1/authors');
    await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
    await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
    await page.getByRole('checkbox', { name: 'Select The Hound of the Baskervilles' }).check();
    await page.getByTestId('selection-send').click();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${DIR}/send-1440x900.png` });
  });

  test('settings devices', async ({ page }) => {
    await page.goto('/settings/devices');
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${DIR}/settings-1440x900.png` });
  });
});

test.describe('screenshots @ 390x844', () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('phone authors list', async ({ page }) => {
    await page.goto('/l/1/authors');
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${DIR}/phone-authors-390x844.png` });
  });

  test('phone books', async ({ page }) => {
    await page.goto('/l/1/authors');
    await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
    await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${DIR}/phone-books-390x844.png` });
  });
});

for (const scheme of ['light', 'dark'] as const) {
  test.describe(`ratings & tokens screenshots @ 1440x900 ${scheme}`, () => {
    test.use({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });

    test('ratings columns, filters and details', async ({ page }) => {
      await page.goto('/l/1/authors');
      await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
      await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
      await page.getByRole('button', { name: 'Columns' }).click();
      for (const c of ['My rating', 'Library rating', 'Open Library']) await page.getByRole('menu').getByLabel(c).check();
      await page.getByRole('button', { name: 'Columns' }).click();
      await page.getByTestId('books-sort').selectOption('extRating');
      await page.getByText('The Hound of the Baskervilles').first().click();
      await page.getByTestId('my-rating').getByRole('button', { name: 'Rate 5' }).click();
      await page.getByRole('button', { name: /Filter/ }).click();
      await page.getByTestId('filter-min-lib').selectOption('3');
      await page.waitForTimeout(400);
      await page.screenshot({ path: `${DIR}/ratings-filters-${scheme}-1440x900.png` });
      await page.getByRole('button', { name: /Filter/ }).click();
      await page.waitForTimeout(200);
      await page.screenshot({ path: `${DIR}/ratings-${scheme}-1440x900.png` });
      // a wider table (genre page, author column on): scrolled sideways, checkbox/#/Title stay
      await page.goto('/l/1/genres/2');
      await page.getByRole('button', { name: 'Columns' }).click();
      for (const c of ['My rating', 'Library rating', 'Open Library', 'Genre', 'Size']) await page.getByRole('menu').getByLabel(c, { exact: true }).check();
      await page.getByRole('button', { name: 'Columns' }).click();
      await page.getByTestId('books-sort').selectOption('extRating');
      await page.locator('.scroll .brow .title-btn').nth(2).click();
      await page.waitForTimeout(300);
      await page.locator('.scroll .vlist').evaluate((el) => (el.scrollLeft = el.scrollWidth));
      await page.waitForTimeout(300);
      await page.screenshot({ path: `${DIR}/ratings-scrolled-${scheme}-1440x900.png` });
    });

    test('api tokens', async ({ page }) => {
      await page.goto('/settings/account');
      const box = page.getByTestId('api-tokens');
      await box.getByTestId('token-name').fill('Claude Code on my laptop');
      await box.getByTestId('scope-write').check();
      await box.getByTestId('token-create').click();
      await expect(box.getByTestId('token-secret')).toBeVisible();
      await page.waitForTimeout(300);
      await page.screenshot({ path: `${DIR}/tokens-${scheme}-1440x900.png`, fullPage: true });
    });

    test('devices order and send button', async ({ page }) => {
      await page.goto('/settings/devices');
      await expect(page.getByTestId('device-row').first()).toBeVisible();
      await page.waitForTimeout(200);
      await page.screenshot({ path: `${DIR}/devices-${scheme}-1440x900.png` });
      await page.goto('/l/1/authors');
      await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
      await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
      await page.getByText('The Hound of the Baskervilles').first().click();
      await page.getByRole('button', { name: 'Choose another device' }).click();
      await page.waitForTimeout(200);
      await page.screenshot({ path: `${DIR}/send-button-${scheme}-1440x900.png` });
    });

    test('server settings (ratings, MCP)', async ({ page }) => {
      await page.goto('/settings/server');
      await expect(page.getByTestId('ext-ratings')).toBeVisible();
      await page.waitForTimeout(300);
      await page.screenshot({ path: `${DIR}/server-settings-${scheme}-1440x900.png` });
    });
  });
}

// Start page, search with a correction and editions (docs/web/screenshots/find-*.png)
for (const scheme of ['light', 'dark'] as const) {
  for (const [w, h] of [[1440, 900], [390, 844]] as const) {
    const size = `${w}x${h}`;
    test.describe(`find screenshots @ ${size} ${scheme}`, () => {
      test.use({ viewport: { width: w, height: h }, colorScheme: scheme });

      test('start page', async ({ page }) => {
        await page.goto('/l/1/home');
        await expect(page.getByTestId('home-continue')).toBeVisible();
        await page.waitForTimeout(600);
        await page.screenshot({ path: `${DIR}/find-home-${scheme}-${size}.png` });
      });

      test('search with did-you-mean', async ({ page }) => {
        await page.goto('/l/1/search?q=азимв');
        await expect(page.getByTestId('did-you-mean')).toBeVisible();
        await page.waitForTimeout(400);
        await page.screenshot({ path: `${DIR}/find-didyoumean-${scheme}-${size}.png` });
      });

      test('editions expanded', async ({ page }) => {
        await page.goto('/l/1/search?q=piknik obochine');
        const row = page.getByTestId('search-book').filter({ hasText: 'Пикник на обочине' });
        await row.getByTestId('editions-toggle').click();
        await expect(page.getByTestId('editions-list').getByRole('listitem')).toHaveCount(4);
        if (w > 900) await row.locator('.title').click();
        await page.waitForTimeout(500);
        await page.screenshot({ path: `${DIR}/find-editions-${scheme}-${size}.png` });
      });
    });
  }
}
