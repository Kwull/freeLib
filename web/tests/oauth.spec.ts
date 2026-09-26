import { test, expect } from '@playwright/test';

// OAuth consent page and the authorized apps list (mock backend: mock/server.ts).
const SHOTS = 'test-results/screenshots'; // copied to docs/web/screenshots/oauth-*.png

// one mock backend for the whole file: the screenshots run before the revoke test
test.describe.configure({ mode: 'serial' });

for (const scheme of ['light', 'dark'] as const) {
  test.describe(`oauth screenshots ${scheme}`, () => {
    test.use({ viewport: { width: 1440, height: 900 }, colorScheme: scheme });

    test('consent page', async ({ page }) => {
      await page.goto('/oauth/consent?request=claude');
      await expect(page.getByTestId('oauth-allow')).toBeEnabled();
      await page.waitForTimeout(200);
      await page.screenshot({ path: `${SHOTS}/oauth-consent-${scheme}.png` });
      await page.goto('/oauth/consent?request=local');
      await expect(page.getByTestId('oauth-loopback')).toBeVisible();
      await page.waitForTimeout(200);
      await page.screenshot({ path: `${SHOTS}/oauth-consent-loopback-${scheme}.png` });
    });

    test('authorized apps', async ({ page }) => {
      await page.goto('/settings/account');
      const box = page.getByTestId('api-tokens');
      await expect(box.getByTestId('oauth-app-row').first()).toBeVisible();
      await box.getByTestId('oauth-apps').evaluate((el) => el.scrollIntoView({ block: 'center' }));
      await page.waitForTimeout(200);
      await page.screenshot({ path: `${SHOTS}/oauth-apps-${scheme}.png` });
    });
  });
}


test('consent: verified app, narrowed scopes, redirect back with the code', async ({ page }) => {
  await page.route('https://claude.ai/**', (r) => r.fulfill({ contentType: 'text/html', body: '<p>callback</p>' }));
  await page.goto('/oauth/consent?request=claude');
  const c = page.getByTestId('oauth-consent');
  await expect(c.getByTestId('oauth-app-name')).toHaveText('Claude');
  await expect(c.getByTestId('oauth-verified')).toContainText('claude.ai');
  await expect(c.getByTestId('oauth-redirect-host')).toHaveText('claude.ai');
  await expect(c.getByTestId('oauth-loopback')).toHaveCount(0);
  for (const s of ['read', 'write', 'send']) await expect(c.getByTestId(`oauth-scope-${s}`)).toBeChecked();
  // nothing chosen: Allow is disabled
  for (const s of ['read', 'write', 'send']) await c.getByTestId(`oauth-scope-${s}`).uncheck();
  await expect(c.getByTestId('oauth-allow')).toBeDisabled();
  await c.getByTestId('oauth-scope-read').check();
  const decision = page.waitForRequest((r) => r.url().endsWith('/api/v1/oauth/requests/claude') && r.method() === 'POST');
  await c.getByTestId('oauth-allow').click();
  const body = (await decision).postDataJSON();
  expect(body).toEqual({ approve: true, scopes: ['read'], csrf: 'mock-csrf' });
  await page.waitForURL(/^https:\/\/claude\.ai\/api\/mcp\/auth_callback\?code=mock-code&state=/);
});

test('consent: loopback app warning, deny', async ({ page }) => {
  await page.route('http://127.0.0.1:43210/**', (r) => r.fulfill({ contentType: 'text/html', body: '<p>denied</p>' }));
  await page.goto('/oauth/consent?request=local');
  const c = page.getByTestId('oauth-consent');
  await expect(c.getByTestId('oauth-unverified')).toBeVisible();
  await expect(c.getByTestId('oauth-loopback')).toBeVisible();
  await expect(c.getByTestId('oauth-redirect-host')).toHaveText('127.0.0.1');
  await expect(c.getByTestId('oauth-scope-send')).toHaveCount(0);
  await c.getByTestId('oauth-deny').click();
  await page.waitForURL(/error=access_denied/);
});

test('consent: errors are shown, never redirected', async ({ page }) => {
  await page.goto('/oauth/consent?error=invalid_redirect_uri');
  await expect(page.getByTestId('oauth-error')).toContainText('did not register');
  await page.goto('/oauth/consent?request=expired-one');
  await expect(page.getByTestId('oauth-error')).toContainText('unknown or expired');
});

test('authorized apps: list, connector steps, revoke', async ({ page }) => {
  await page.goto('/settings/account');
  const box = page.getByTestId('api-tokens');
  await expect(box.getByTestId('oauth-connect')).toContainText('Add custom connector');
  await expect(box.getByTestId('oauth-connect')).toContainText('/mcp');
  const apps = box.getByTestId('oauth-apps');
  const claude = apps.getByTestId('oauth-app-row').filter({ hasText: 'Claude' }).first();
  await expect(claude).toContainText('claude.ai');
  await expect(claude).toContainText('send');
  const script = apps.getByTestId('oauth-app-row').filter({ hasText: 'My MCP script' });
  await expect(script).toContainText('unverified name');
  await expect(script).toContainText('127.0.0.1');
  // the audit log names the app
  await expect(box.getByTestId('token-audit')).toContainText('Claude');
  page.once('dialog', (d) => d.accept());
  await script.getByRole('button', { name: 'Revoke' }).click();
  await expect(apps.getByTestId('oauth-app-row').filter({ hasText: 'My MCP script' })).toHaveCount(0);
});

