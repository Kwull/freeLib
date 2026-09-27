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

/** A row of Settings → Users by its user name (other rows' "Merge into…" menus list every name). */
const userRow = (page: import('@playwright/test').Page, name: string) =>
  page.locator('.device-row').filter({ has: page.locator('.name', { hasText: new RegExp(`^${name.replace(/[()]/g, '\\$&')}$`) }) });
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
  await expect(userRow(page, 'alice')).toContainText('SSO');
  await expect(userRow(page, 'alice')).toContainText('no password');
});

test('a taken user name: link it with the password, never a silent "name (2)"', async ({ browser }) => {
  // the admin creates a local account "kwull"
  const admin = await browser.newPage();
  await admin.goto(`${BASE}/`);
  await admin.getByLabel('Username').fill('admin');
  await admin.getByLabel('Password').fill('admin-e2e');
  await admin.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(admin.getByTestId('first-run')).toBeVisible();
  const r = await admin.request.post(`${BASE}/api/v1/users`, { data: { username: 'kwull', password: 'kwull-pass', role: 'reader' } });
  expect(r.ok()).toBe(true);

  // kwull's first single sign-on: asked for the password of the existing account
  idp.user = { sub: 'e2e-kwull', preferred_username: 'kwull', email: 'kwull@example.org' };
  const page = await (await browser.newContext()).newPage();
  await page.goto(`${BASE}/`);
  await page.getByTestId('sso-button').click();
  await expect(page.getByTestId('sso-link-notice')).toContainText('An account “kwull” already exists. Sign in with its password to link single sign-on.', { timeout: 15000 });
  await expect(page.getByLabel('Username')).toHaveValue('kwull');
  await page.getByLabel('Password').fill('kwull-pass');
  await page.getByRole('button', { name: 'Sign in and link' }).click();
  await expect(page.getByTestId('first-run')).toBeVisible();
  await page.goto(`${BASE}/settings/account`);
  await expect(page.getByTestId('account-user')).toContainText('kwull');
  await expect(page.getByTestId('account-sso')).toContainText('Linked');
  // no duplicate was made
  await admin.goto(`${BASE}/settings/users`);
  await expect(userRow(admin, 'kwull')).toHaveCount(1);
  await admin.close();
  await page.close();
});

test('the admin merges a duplicate account into the original', async ({ browser }) => {
  const admin = await (await browser.newContext()).newPage();
  await admin.goto(`${BASE}/`);
  await admin.getByLabel('Username').fill('admin');
  await admin.getByLabel('Password').fill('admin-e2e');
  await admin.getByRole('button', { name: 'Sign in', exact: true }).click();
  await expect(admin.getByTestId('first-run')).toBeVisible();
  expect((await admin.request.post(`${BASE}/api/v1/users`, { data: { username: 'dora', password: 'dora-pass', role: 'reader' } })).ok()).toBe(true);

  // a duplicate "dora (2)", made knowingly (older versions made it silently)
  idp.user = { sub: 'e2e-dora', preferred_username: 'dora' };
  const dup = await (await browser.newContext()).newPage();
  await dup.goto(`${BASE}/`);
  await dup.getByTestId('sso-button').click();
  await dup.getByTestId('sso-create-separate').click();
  await expect(dup.getByTestId('first-run')).toBeVisible({ timeout: 15000 });
  await dup.goto(`${BASE}/settings/account`);
  await expect(dup.getByTestId('account-user')).toContainText('dora (2)');
  await dup.close();

  // Settings → Users → "Merge into…" on the duplicate's row
  await admin.goto(`${BASE}/settings/users`);
  const row = userRow(admin, 'dora (2)');
  await expect(row).toContainText('SSO');
  admin.once('dialog', (d) => { expect(d.message()).toContain('Merge “dora (2)” into “dora”'); void d.accept(); });
  await row.getByTestId('merge-into').selectOption({ label: 'dora' });
  await expect(userRow(admin, 'dora (2)')).toHaveCount(0);
  await expect(userRow(admin, 'dora')).toContainText('SSO');
  await admin.close();

  // single sign-on is dora now
  const page = await (await browser.newContext()).newPage();
  await page.goto(`${BASE}/`);
  await page.getByTestId('sso-button').click();
  await expect(page.getByTestId('first-run')).toBeVisible({ timeout: 15000 });
  await page.goto(`${BASE}/settings/account`);
  await expect(page.getByTestId('account-user')).toContainText('dora');
  await expect(page.getByTestId('account-user')).not.toContainText('(2)');
  await page.close();
});
