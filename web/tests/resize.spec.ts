import { test, expect } from '@playwright/test';

test.use({ viewport: { width: 1440, height: 900 } });

async function drag(page: import('@playwright/test').Page, handle: import('@playwright/test').Locator, dx: number) {
  const b = (await handle.boundingBox())!;
  const x = b.x + b.width / 2, y = b.y + Math.min(b.height / 2, 200);
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y, { steps: 4 });
  await page.mouse.move(x + dx, y, { steps: 4 });
  await page.mouse.up();
}

test('panes and columns resize by drag and keyboard, and the widths persist', async ({ page }) => {
  await page.goto('/');
  await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
  await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
  await page.getByText('The Hound of the Baskervilles').first().click();

  const list = page.getByRole('region', { name: 'Authors' }).or(page.locator('section.browser'));
  const listW = async () => (await page.locator('section.browser').boundingBox())!.width;
  const w0 = await listW();
  const listHandle = page.getByRole('separator', { name: 'Resize the list' });
  await drag(page, listHandle, 80);
  await expect.poll(listW).toBeGreaterThan(w0 + 60);
  expect(list).toBeTruthy();

  // details pane: dragging left widens it
  const details = page.getByRole('complementary', { name: 'Details' });
  const dW0 = (await details.boundingBox())!.width;
  await drag(page, page.getByRole('separator', { name: 'Resize the details pane' }), -60);
  await expect.poll(async () => (await details.boundingBox())!.width).toBeGreaterThan(dW0 + 40);

  // a column (Size, right of the title): its left edge; dragging left widens it
  const sizeHandle = page.getByRole('separator', { name: 'Resize column Size' });
  const before = Number(await sizeHandle.getAttribute('aria-valuenow'));
  await drag(page, sizeHandle, -40);
  await expect.poll(async () => Number(await sizeHandle.getAttribute('aria-valuenow'))).toBeGreaterThan(before + 20);
  const sizeW = Number(await sizeHandle.getAttribute('aria-valuenow'));

  // keyboard: arrows step, Enter resets
  const addedHandle = page.getByRole('separator', { name: 'Resize column Added' });
  await addedHandle.focus();
  const a0 = Number(await addedHandle.getAttribute('aria-valuenow'));
  await page.keyboard.press('ArrowLeft');
  await expect(addedHandle).toHaveAttribute('aria-valuenow', String(a0 + 16));

  // header and rows share the grid template
  const headTpl = await page.locator('.brow.head').evaluate((el) => getComputedStyle(el).gridTemplateColumns);
  const rowTpl = await page.locator('.scroll .brow').first().evaluate((el) => getComputedStyle(el).gridTemplateColumns);
  expect(rowTpl).toBe(headTpl);

  const listAfter = await listW();
  await page.waitForTimeout(1200); // prefs are saved debounced
  await page.reload();
  await page.getByText('The Hound of the Baskervilles').first().click();
  await expect.poll(listW).toBeGreaterThan(listAfter - 2);
  await expect(page.getByRole('separator', { name: 'Resize column Size' })).toHaveAttribute('aria-valuenow', String(sizeW));
  await expect(page.getByRole('separator', { name: 'Resize column Added' })).toHaveAttribute('aria-valuenow', String(a0 + 16));

  // double click resets to the default width
  await page.getByRole('separator', { name: 'Resize the list' }).dblclick();
  await expect.poll(listW).toBeLessThan(listAfter - 40);
});
