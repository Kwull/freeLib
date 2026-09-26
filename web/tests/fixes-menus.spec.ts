import { test, expect, devices, type Page, type Locator } from '@playwright/test';

// Every dropdown, menu and popover closes on an outside click, on Escape (focus back on its
// trigger), when another one opens, and on navigation (utils/dismiss.ts).
test.use({ viewport: { width: 1600, height: 900 }, colorScheme: 'dark' });

async function openDoyle(page: Page) {
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
  await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
  await expect(page.getByText('The Hound of the Baskervilles').first()).toBeVisible({ timeout: 15000 });
  await page.getByText('The Hound of the Baskervilles').first().click();
  await expect(page.getByTestId('quick-send')).toBeVisible();
}

type Menu = { name: string; trigger: (p: Page) => Locator; popup: (p: Page) => Locator; setup?: (p: Page) => Promise<void> };

const MENUS: Menu[] = [
  { name: 'Columns', trigger: (p) => p.getByRole('button', { name: 'Columns' }), popup: (p) => p.getByTestId('columns-menu') },
  { name: 'Filter', trigger: (p) => p.locator('.filter-chooser > button'), popup: (p) => p.getByTestId('filter-menu') },
  { name: 'library picker', trigger: (p) => p.locator('.lib-switch > button'), popup: (p) => p.locator('.lib-switch [role=menu]') },
  { name: 'account', trigger: (p) => p.getByRole('button', { name: 'Account' }), popup: (p) => p.locator('.account [role=menu]') },
  { name: 'download', trigger: (p) => p.getByRole('button', { name: 'Download as' }), popup: (p) => p.locator('.dl-wrap [role=menu]') },
  { name: 'send to device', trigger: (p) => p.getByRole('button', { name: 'Other device' }), popup: (p) => p.locator('.dev-menu') },
  { name: 'activity', trigger: (p) => p.getByRole('button', { name: 'Activity' }), popup: (p) => p.getByRole('complementary', { name: 'Activity' }) },
];

for (const m of MENUS) {
  test(`${m.name}: closes on an outside click and on Escape`, async ({ page }) => {
    await openDoyle(page);
    await m.setup?.(page);
    const trigger = m.trigger(page);
    const popup = m.popup(page);

    await trigger.click();
    await expect(popup).toBeVisible();
    // outside: the page header of the books pane (the activity panel has a scrim there)
    await page.mouse.click(700, 110);
    await expect(popup).toBeHidden();

    await trigger.click();
    await expect(popup).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(popup).toBeHidden();
    if (m.name !== 'activity') await expect(trigger).toBeFocused();

    // the trigger toggles it closed as well
    await trigger.click();
    await expect(popup).toBeVisible();
    if (m.name !== 'activity') {
      await trigger.click();
      await expect(popup).toBeHidden();
    } else {
      await page.keyboard.press('Escape');
    }
  });
}

test('opening another menu closes the open one; Columns stays open while toggling', async ({ page }) => {
  await openDoyle(page);
  const columns = page.getByTestId('columns-menu');
  await page.getByRole('button', { name: 'Columns' }).click();
  await expect(columns).toBeVisible();
  // multi-toggle: stays open
  await columns.getByLabel('Genre').check();
  await columns.getByLabel('Genre').uncheck();
  await expect(columns).toBeVisible();
  // another menu (opened by keyboard, so no outside press is involved)
  await page.getByRole('button', { name: 'Download as' }).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.dl-wrap [role=menu]')).toBeVisible();
  await expect(columns).toBeHidden();
  await page.getByRole('button', { name: 'Account' }).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.account [role=menu]')).toBeVisible();
  await expect(page.locator('.dl-wrap [role=menu]')).toBeHidden();
});

test('navigation closes menus (links, back/forward)', async ({ page }) => {
  await openDoyle(page);
  const account = page.locator('.account [role=menu]');
  await page.getByRole('button', { name: 'Account' }).click();
  await expect(account).toBeVisible();
  await page.getByRole('menu').getByRole('button', { name: 'Settings' }).click();
  await expect(page).toHaveURL(/\/settings/);
  await expect(account).toBeHidden();
  // back (popstate) with the library menu open
  await page.locator('.lib-switch > button').focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.lib-switch [role=menu]')).toBeVisible();
  await page.goBack();
  await expect(page).toHaveURL(/\/authors\//);
  await expect(page.locator('.lib-switch [role=menu]')).toBeHidden();
});

test('co-authors popover closes the same way', async ({ page }) => {
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill('Asimov Isaac');
  await page.getByRole('button', { name: /Asimov Isaac/ }).first().click();
  const more = page.locator('.scope-header').getByRole('button', { name: /and \d+ more/ });
  const pop = page.getByRole('dialog', { name: 'Co-authors' });
  await more.click();
  await expect(pop).toBeVisible();
  await page.mouse.click(900, 600);
  await expect(pop).toBeHidden();
  await more.click();
  await expect(pop).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(pop).toBeHidden();
  await expect(more).toBeFocused();
  // opening the Filter menu closes the popover
  await more.click();
  await expect(pop).toBeVisible();
  await page.locator('.filter-chooser > button').focus();
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('filter-menu')).toBeVisible();
  await expect(pop).toBeHidden();
});

test.describe('phone', () => {
  const { defaultBrowserType: _b, ...iphone } = devices['iPhone 13'];
  test.use({ ...iphone });

  test('selection "more" menu and the filter menu close on an outside tap and on Escape', async ({ page }) => {
    await page.goto('/l/1/authors');
    await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
    await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
    await page.locator('.m-row input[type=checkbox]').first().check();
    const more = page.getByRole('button', { name: 'More' });
    const menu = page.getByRole('menu', { name: 'More' });
    await more.click();
    await expect(menu).toBeVisible();
    await page.locator('.scope-header-phone .ph-info').tap();
    await expect(menu).toBeHidden();
    await more.click();
    await expect(menu).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(menu).toBeHidden();
    // opening the filter menu closes "more"
    await more.click();
    await page.locator('.filter-chooser > button').tap();
    await expect(page.getByTestId('filter-menu')).toBeVisible();
    await expect(menu).toBeHidden();
    await page.locator('.scope-header-phone .ph-info').tap();
    await expect(page.getByTestId('filter-menu')).toBeHidden();
  });
});
