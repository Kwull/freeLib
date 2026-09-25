import { test, expect, type Page } from '@playwright/test';

// "Asimov Isaac" in the mock: ~320 live books in ~20 series, ~150 anthologies with 5–30
// authors each (thousands of names share a book with him), one real co-author.
async function openAsimov(page: Page) {
  await page.goto('/');
  await page.getByLabel('Filter authors').fill('Asimov Isaac');
  await page.getByRole('button', { name: /Asimov Isaac/ }).first().click();
  await expect(page.getByRole('heading', { level: 1, name: 'Asimov Isaac' })).toBeVisible();
}

test.use({ viewport: { width: 1440, height: 900 } });

test('header stays compact: top co-authors plus a searchable "more" list', async ({ page }) => {
  await openAsimov(page);
  const header = page.locator('.scope-header');
  await expect(header.locator('.coauthors a')).toHaveCount(3);
  // the real co-author comes first
  await expect(header.locator('.coauthors a').first()).toHaveText('Silverberg Robert');
  const box = await header.boundingBox();
  expect(box!.height).toBeLessThan(170);

  await header.getByRole('button', { name: /and \d+ more/ }).click();
  const pop = page.getByRole('dialog', { name: 'Co-authors' });
  await expect(pop).toBeVisible();
  await expect(pop.getByText(/names share a book/)).toBeVisible();
  await pop.getByLabel('Find a co-author').fill('silverb');
  await expect(pop.locator('.r')).toHaveCount(1);
  await expect(pop.locator('.r .tag')).toHaveText('co-author');
  await page.keyboard.press('Escape');
  await expect(pop).toBeHidden();
});

test('many series start collapsed; expand/collapse all, filter, anthologies, sort', async ({ page }) => {
  await openAsimov(page);
  const groups = page.locator('.group-head');
  await expect(groups.first()).toBeVisible();
  // collapsed: only group rows, no book rows
  await expect(page.locator('.scroll .brow')).toHaveCount(0);
  await expect(groups.first().locator('.gcount')).toHaveText(/\d+/);

  await page.getByRole('button', { name: 'Expand all' }).click();
  await expect(page.locator('.scroll .brow').first()).toBeVisible();
  await page.getByRole('button', { name: 'Collapse all' }).click();
  await expect(page.locator('.scroll .brow')).toHaveCount(0);

  // one group opens on click
  await groups.first().locator('.gtoggle').click();
  await expect(page.locator('.scroll .brow').first()).toBeVisible();

  // the text filter searches all his books and opens the groups
  await page.getByPlaceholder('Find in these books').fill('foundation and empire');
  await expect(page.locator('.scroll .brow', { hasText: 'Foundation and Empire' })).toBeVisible();
  await expect(page.locator('.toolbar .shown')).toHaveText(/^\d+ of \d+$/);
  await page.getByPlaceholder('Find in these books').fill('');

  // anthologies (≥ 4 authors) can be hidden; the button says how many
  const anth = page.getByRole('button', { name: /Hide anthologies \(\d+\)/ });
  const n = Number((await anth.textContent())!.match(/\((\d+)\)/)![1]);
  expect(n).toBeGreaterThan(100);
  await anth.click();
  await expect(page.getByRole('button', { name: `${n} anthologies hidden` })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.toolbar .shown')).toBeVisible();
  await page.getByRole('button', { name: `${n} anthologies hidden` }).click();

  // sorting by title shows a flat list
  await page.getByLabel('Sort').selectOption('title');
  await expect(page.locator('.group-head')).toHaveCount(0);
  await expect(page.locator('.scroll .brow').first()).toBeVisible();
  await page.getByLabel('Sort').selectOption('series');
});

test('details pane shows the author summary until a book is picked, and can be collapsed', async ({ page }) => {
  await openAsimov(page);
  const details = page.getByRole('complementary', { name: 'Details' });
  await expect(details.getByText('About the author')).toBeVisible();
  await expect(details.getByRole('link', { name: 'Foundation' }).first()).toBeVisible();
  await expect(details.getByRole('link', { name: 'Silverberg Robert' })).toBeVisible();

  await details.getByRole('button', { name: 'Hide details' }).click();
  await expect(page.getByRole('button', { name: 'Show details' })).toBeVisible();
  await page.getByRole('button', { name: 'Show details' }).click();
  await expect(details.getByText('About the author')).toBeVisible();
});

test('an author without series has no group headers', async ({ page }) => {
  await page.goto('/');
  await page.getByLabel('Filter authors').fill('Lem Stanis');
  await page.getByRole('button', { name: /Lem Stanisław/ }).first().click();
  await expect(page.locator('.scroll .brow').first()).toBeVisible();
  await expect(page.locator('.group-head')).toHaveCount(0);
});
