import { test, expect } from '@playwright/test';

test('API tokens: create with scopes, copy-paste configs, revoke, audit', async ({ page }) => {
  await page.goto('/settings/account');
  const box = page.getByTestId('api-tokens');
  await expect(box).toBeVisible({ timeout: 15000 });
  await expect(box.getByTestId('mcp-url')).toHaveText(/\/mcp$/);
  await expect(box.getByTestId('token-row').filter({ hasText: 'Claude Desktop' })).toHaveCount(1);
  await expect(box.getByTestId('token-audit')).toContainText('send_books');

  await box.getByTestId('token-name').fill('Laptop e2e');
  await box.getByTestId('scope-send').check();
  await box.getByTestId('token-create').click();
  const secret = box.getByTestId('token-secret').locator('code');
  await expect(secret).toHaveText(/^fl_[A-Za-z0-9_-]{43}$/);
  const s = await secret.textContent();
  await expect(box.getByTestId('snippet-code')).toContainText(`claude mcp add --transport http freelib`);
  await expect(box.getByTestId('snippet-code')).toContainText(`Authorization: Bearer ${s}`);
  await box.getByRole('tab', { name: 'Claude Desktop' }).click();
  await expect(box.getByTestId('snippet-desktop')).toContainText('mcp-remote');
  await expect(box.getByTestId('snippet-desktop')).toContainText(`"FREELIB_AUTH": "Bearer ${s}"`);
  await box.getByRole('tab', { name: 'Other clients' }).click();
  await expect(box.getByTestId('snippet-other')).toContainText('"type": "http"');
  const row = box.getByTestId('token-row').filter({ hasText: 'Laptop e2e' });
  await expect(row).toContainText('read');
  await expect(row).toContainText('send');
  await expect(row).toContainText(s!.slice(0, 11));

  page.once('dialog', (d) => d.accept());
  await row.getByRole('button', { name: 'Revoke' }).click();
  await expect(box.getByTestId('token-row').filter({ hasText: 'Laptop e2e' })).toHaveCount(0);
  await expect(box.getByTestId('token-secret')).toHaveCount(0);
});

test('server settings: MCP and external ratings switches with progress', async ({ page }) => {
  await page.goto('/settings/server');
  await expect(page.getByTestId('mcp-enabled')).toBeChecked({ timeout: 15000 });
  const er = page.getByTestId('ext-ratings');
  await expect(er.getByTestId('ext-enabled')).toBeChecked();
  await expect(er.getByTestId('ext-progress')).toContainText('books looked up');
  await expect(er.getByRole('progressbar')).toBeVisible();
  await expect(er).toContainText('openlibrary.org');
  await page.goto('/libraries');
  await expect(page.getByTestId('lib-ext-ratings').first()).toContainText('Open Library:');
});
