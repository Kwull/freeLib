import { test, expect } from '@playwright/test';
import { spawn, type ChildProcess } from 'node:child_process';
import { mkdtempSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { startFakeOidc, type FakeOidc } from './fake-oidc';

// Single sign-on end to end: a second release server (the one `start-server.sh` built) with
// FREELIB_OIDC_* pointing at an in-test fake provider, driven through the real web app.
const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '../..');
const BIN = path.join(ROOT, 'server/target/release/freelib-server');
const PORT = Number(process.env.FREELIB_SSO_E2E_PORT ?? 8098);
const BASE = `http://127.0.0.1:${PORT}`;

let idp: FakeOidc;
let server: ChildProcess;
let dir: string;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  test.skip(!existsSync(BIN), `${BIN} is not built`);
  idp = await startFakeOidc('freelib-e2e');
  dir = mkdtempSync(path.join(tmpdir(), 'freelib-sso-e2e-'));
  server = spawn(BIN, [], {
    env: {
      ...process.env,
      FREELIB_PORT: String(PORT),
      FREELIB_BIND: '127.0.0.1',
      FREELIB_DATA_DIR: path.join(dir, 'data'),
      FREELIB_CACHE_DIR: path.join(dir, 'cache'),
      FREELIB_BOOKS_DIR: path.join(dir, 'books'),
      FREELIB_EXPORT_DIR: path.join(dir, 'export'),
      FREELIB_WEB_DIR: path.join(ROOT, 'web/dist'),
      FREELIB_CALIBRE: 'none',
      FREELIB_ADMIN_PASSWORD: 'admin-e2e',
      FREELIB_PUBLIC_URL: BASE,
      FREELIB_OIDC_ISSUER: idp.issuer,
      FREELIB_OIDC_CLIENT_ID: 'freelib-e2e',
      FREELIB_OIDC_BUTTON: 'Sign in with Pocket ID',
      FREELIB_OIDC_ADMIN_GROUP: 'freelib-admins',
      FREELIB_AUTOIMPORT: '',
    },
    stdio: 'inherit',
  });
  const deadline = Date.now() + 30_000;
  for (;;) {
    try {
      const r = await fetch(`${BASE}/api/v1/session`);
      if (r.ok) break;
    } catch { /* not up yet */ }
    if (Date.now() > deadline) throw new Error('SSO test server did not start');
    await new Promise((r) => setTimeout(r, 200));
  }
});

test.afterAll(async () => {
  server?.kill('SIGTERM');
  await idp?.close();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test('sign in with SSO: account created, admin by group, linked in Settings → Account', async ({ page }) => {
  idp.user = { sub: 'e2e-alice', preferred_username: 'alice', email: 'alice@example.org', groups: ['freelib-admins'] };
  idp.error = null;
  await page.goto(`${BASE}/l/1/authors`);
  const button = page.getByTestId('sso-button');
  await expect(button).toHaveText(/Sign in with Pocket ID/);
  await expect(page.getByLabel('Username')).toBeVisible();
  await button.click();
  // back in the app, signed in; no libraries yet and alice is an admin by group
  await expect(page.getByTestId('first-run')).toBeVisible({ timeout: 15000 });
  await expect(page.getByRole('link', { name: 'Add your first library' })).toBeVisible();
  await page.goto(`${BASE}/settings/account`);
  await expect(page.getByTestId('account-user')).toContainText('alice');
  await expect(page.getByTestId('account-sso')).toContainText('alice@example.org');
  await expect(page.getByTestId('account-password')).toContainText('OPDS');
});

test('a refused sign-in is explained on the login page', async ({ page }) => {
  idp.error = 'access_denied';
  await page.goto(`${BASE}/`);
  await page.getByTestId('sso-button').click();
  await expect(page.getByTestId('sso-error')).toContainText('refused or cancelled');
  idp.error = null;
});

test('local admin links SSO from Settings → Account', async ({ page }) => {
  idp.user = { sub: 'e2e-admin', preferred_username: 'someone-else' };
  await page.goto(`${BASE}/`);
  await page.getByLabel('Username').fill('admin');
  await page.getByLabel('Password').fill('admin-e2e');
  await page.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(page.getByTestId('first-run')).toBeVisible();
  await page.goto(`${BASE}/settings/account`);
  await page.getByRole('button', { name: 'Link single sign-on' }).click();
  // provider → callback → back to Settings → Account
  await expect(page.getByTestId('account-sso')).toContainText('Linked', { timeout: 15000 });
  expect(page.url()).toContain('/settings/account');
  await expect(page.getByRole('button', { name: 'Unlink' })).toBeEnabled();
  // the admin sees both accounts' links in Users
  await page.goto(`${BASE}/settings/users`);
  await expect(page.locator('.device-row', { hasText: 'alice' })).toContainText('SSO');
  await expect(page.locator('.device-row', { hasText: 'alice' })).toContainText('no password');
});
