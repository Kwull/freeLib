import { test, expect, type Page } from '@playwright/test';

async function names(page: Page) {
  return page.getByTestId('device-row').locator('.name').allTextContents();
}

test('device order: arrows and drag and drop; the first device is the default everywhere', async ({ page }) => {
  await page.goto('/settings/devices');
  await expect(page.getByTestId('device-row').first()).toBeVisible({ timeout: 15000 });
  const initial = await names(page);
  expect(initial[0]).toBe('Kindle');
  try {
    // keyboard-accessible buttons
    await page.getByRole('button', { name: 'Move Kindle (USB) up' }).click();
    await expect.poll(() => names(page)).toEqual(['Kindle (USB)', 'Kindle', ...initial.slice(2)]);
    await expect(page.getByTestId('device-row').first()).toContainText('default');
    await expect(page.getByRole('button', { name: 'Move Kindle (USB) up' })).toBeDisabled();
    // drag "Kobo" onto the first row
    const kobo = page.getByTestId('device-row').filter({ hasText: 'Kobo' }).locator('.handle');
    await kobo.dragTo(page.getByTestId('device-row').first(), { targetPosition: { x: 20, y: 4 } });
    await expect.poll(async () => (await names(page))[0]).toBe('Kobo');
    // persisted on the server
    await page.reload();
    await expect.poll(async () => (await names(page))[0]).toBe('Kobo');

    // the details pane: plain verb by kind, device as a caption, other devices in the menu
    await page.goto('/l/1/authors');
    await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
    await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
    await page.getByText('The Hound of the Baskervilles').first().click();
    await expect(page.getByTestId('quick-send')).toHaveText('Download');
    await expect(page.getByTestId('send-caption')).toHaveText('Kobo · KEPUB');
    await page.getByRole('button', { name: 'Choose another device' }).click();
    const items = page.getByRole('menuitem');
    await expect(items.first()).toContainText('Kobo');
    await items.filter({ hasText: 'Kindle · EPUB' }).click();
    const dlg = page.getByRole('dialog');
    await expect(dlg.getByRole('button', { name: 'Send 1 book' })).toBeVisible();
    await dlg.getByRole('button', { name: 'Cancel' }).click();
    // the selection bar and the Send dialog follow the order too
    await page.getByRole('checkbox', { name: 'Select The White Company' }).check();
    await expect(page.getByTestId('selection-send')).toHaveText('Download…');
    await page.getByTestId('selection-send').click();
    const cards = page.getByRole('dialog').locator('.device-card, [role=radio]');
    await expect(page.getByRole('dialog').getByRole('button', { name: 'Download 1 book' })).toBeVisible();
    if (await cards.count()) await expect(cards.first()).toContainText('Kobo');
  } finally {
    // the mock is shared by parallel tests: restore the order
    const ids = await page.evaluate(async () => (await (await fetch('/api/v1/devices')).json()).map((d: { id: number }) => d.id).sort((a: number, b: number) => a - b));
    await page.evaluate(async (ids) => { await fetch('/api/v1/devices/order', { method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ids }) }); }, ids);
  }
});
