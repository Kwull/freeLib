import { test, expect, type Page, type BrowserContext } from '@playwright/test';

// Startup, importing, error and first-run states against the mock's simulated server states
// (mock/scenario.ts), with screenshots in light and dark, desktop and phone.
const DIR = 'test-results/screenshots/states';

async function scenario(context: BrowserContext, s: string, extra = '') {
  const id = Math.random().toString(36).slice(2);
  await context.addCookies([{
    name: 'freelib_mock',
    value: encodeURIComponent(`s=${s}&id=${id}${extra}`),
    url: 'http://localhost:5183',
  }]);
}

const VIEWS = [
  { name: 'desktop', viewport: { width: 1440, height: 900 } },
  { name: 'phone', viewport: { width: 390, height: 844 } },
] as const;
const THEMES = ['light', 'dark'] as const;

async function shot(page: Page, name: string) {
  await page.waitForTimeout(250);
  await page.screenshot({ path: `${DIR}/${name}.png` });
}

/** Never the old bare, left-aligned "Loading…" text. */
async function noBareLoading(page: Page) {
  await expect(page.locator('.boot')).toHaveCount(0);
}

for (const v of VIEWS) {
  for (const theme of THEMES) {
    test.describe(`${v.name} ${theme}`, () => {
      test.use({ viewport: v.viewport, colorScheme: theme });
      const tag = `${v.name}-${theme}`;

      test('session loading: the app frame with skeletons', async ({ page, context }) => {
        await scenario(context, 'sessionhang');
        await page.goto('/');
        await expect(page.getByTestId('shell-skeleton')).toBeVisible();
        await expect(page.getByTestId('browse-skeleton')).toBeVisible();
        await noBareLoading(page);
        await shot(page, `boot-${tag}`);
      });

      test('rebuild after an update: progress, other library, continues by itself', async ({ page, context }) => {
        await scenario(context, 'rebuild', '&dur=7000');
        await page.goto('/');
        const card = page.getByTestId('library-importing');
        await expect(card).toBeVisible();
        await expect(card.getByRole('heading', { name: 'Rebuilding the catalog' })).toBeVisible();
        await expect(card).toContainText('about a minute');
        await expect(card.getByRole('progressbar')).toBeVisible();
        await expect(card.getByRole('button', { name: 'Home Collection' })).toBeVisible();
        // the shell stays: library switcher and navigation
        await expect(page.getByRole('button', { name: /Flibusta/ })).toBeVisible();
        await expect(page.locator('nav[aria-label="Main"]:visible')).toHaveCount(1);
        await noBareLoading(page);
        await expect(card).toContainText(/\d+%/);
        await shot(page, `rebuild-${tag}`);
        // done: the authors list appears without a reload
        await expect(page.getByLabel('Filter authors')).toBeVisible({ timeout: 15000 });
        await expect(card).toHaveCount(0);
        await expect(page.locator('.arow').first()).toBeVisible();
      });

      test('import failed: server message and retry', async ({ page, context }) => {
        await scenario(context, 'importfail');
        await page.goto('/l/1/authors');
        const card = page.getByTestId('library-error');
        await expect(card).toBeVisible();
        await expect(card).toContainText('No such file or directory');
        await expect(card.getByRole('button', { name: 'Retry import' })).toBeVisible();
        await shot(page, `import-error-${tag}`);
      });

      test('libraries cannot be loaded: error card with retry', async ({ page, context }) => {
        await scenario(context, 'libserror');
        await page.goto('/');
        const card = page.getByTestId('libraries-error');
        await expect(card).toBeVisible();
        await expect(card).toContainText('disk I/O error');
        await expect(card.getByRole('button', { name: 'Retry' })).toBeVisible();
        await shot(page, `libraries-error-${tag}`);
      });

      test('first run: add your first library', async ({ page, context }) => {
        await scenario(context, 'nolibs');
        await page.goto('/');
        const card = page.getByTestId('first-run');
        await expect(card).toBeVisible();
        await shot(page, `first-run-${tag}`);
        await card.getByRole('link', { name: 'Add your first library' }).click();
        await expect(page).toHaveURL(/\/libraries/);
        await expect(page.getByRole('dialog')).toBeVisible();
      });

      test('login page with single sign-on', async ({ page, context }) => {
        await scenario(context, 'loggedout,sso');
        await page.goto('/');
        await expect(page.getByTestId('sso-button')).toHaveText(/Sign in with Pocket ID/);
        await expect(page.getByLabel('Username')).toBeVisible();
        await shot(page, `login-sso-${tag}`);
      });
    });
  }
}

test.describe('more states (desktop)', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('first import without SSE: polling finishes it', async ({ page, context }) => {
    await scenario(context, 'importing,nosse', '&dur=3000');
    await page.goto('/l/1/authors');
    const card = page.getByTestId('library-importing');
    await expect(card.getByRole('heading', { name: 'Importing library…' })).toBeVisible();
    await expect(page.getByLabel('Filter authors')).toBeVisible({ timeout: 15000 });
  });

  test('session error: boot card with retry', async ({ page, context }) => {
    await scenario(context, 'sessionerror');
    await page.goto('/');
    const card = page.getByTestId('boot-error');
    await expect(card).toContainText('database is locked');
    await expect(card.getByRole('button', { name: 'Retry' })).toBeVisible();
    await shot(page, 'boot-error-desktop-light');
    // the server recovers
    await context.clearCookies();
    await card.getByRole('button', { name: 'Retry' }).click();
    await expect(page.getByLabel('Filter authors')).toBeVisible();
  });

  test('reader without libraries gets a note', async ({ page, context }) => {
    await scenario(context, 'nolibs,reader');
    await page.goto('/');
    await expect(page.getByTestId('first-run')).toContainText('An administrator has to add a library');
    await expect(page.getByRole('link', { name: 'Add your first library' })).toHaveCount(0);
  });

  test('big authors list: progress while it downloads, IndexedDB afterwards', async ({ page, context }) => {
    await scenario(context, 'slow', '&slow=3000');
    await page.goto('/l/1/authors');
    const progress = page.getByTestId('list-progress');
    await expect(progress).toBeVisible();
    await expect(progress).toContainText(/of [\d,]+ · [\d.]+ MB/);
    await shot(page, 'authors-progress-desktop-light');
    await expect(page.locator('.arow').first()).toBeVisible({ timeout: 15000 });
    // second visit: served from IndexedDB, no progress
    await page.reload();
    await expect(page.locator('.arow').first()).toBeVisible({ timeout: 3000 });
  });

  test('SSO error is explained on the login page; password form can be disabled', async ({ page, context }) => {
    await scenario(context, 'loggedout,sso,nopassword');
    await page.goto('/');
    await expect(page.getByLabel('Username')).toHaveCount(0);
    await page.getByTestId('sso-button').click();
    await expect(page).toHaveURL(/ssoError=provider/);
    await expect(page.getByTestId('sso-error')).toContainText('refused or cancelled');
    await shot(page, 'login-sso-error-desktop-light');
    await page.getByRole('button', { name: /local administrator/ }).click();
    await expect(page.getByLabel('Username')).toBeVisible();
  });

  test('account settings: link SSO, set a password for OPDS', async ({ page, context }) => {
    await scenario(context, 'sso,reader,ssouser');
    await page.goto('/settings/account');
    await expect(page.getByTestId('account-sso')).toContainText('reader@example.org');
    await expect(page.getByTestId('account-password')).toContainText('OPDS');
    await expect(page.getByRole('button', { name: 'Unlink' })).toBeDisabled();
    await shot(page, 'account-sso-desktop-light');
  });
});
