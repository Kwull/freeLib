import { test } from '@playwright/test';

const DIR = 'test-results/screenshots';

test.describe('screenshots @ 1440x900', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('main (authors + books + details)', async ({ page }) => {
    await page.goto('/');
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
    await page.goto('/');
    await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
    await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
    await page.getByRole('checkbox', { name: 'Select The Hound of the Baskervilles' }).check();
    await page.getByRole('button', { name: 'Send to…' }).click();
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
    await page.goto('/');
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${DIR}/phone-authors-390x844.png` });
  });

  test('phone books', async ({ page }) => {
    await page.goto('/');
    await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
    await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
    await page.waitForTimeout(400);
    await page.screenshot({ path: `${DIR}/phone-books-390x844.png` });
  });
});
