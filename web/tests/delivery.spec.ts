import { test, expect, type Page } from '@playwright/test';

// Delivery: "Send to my phone" (QR), "Open in Books" on iPhone, whole-series sends and
// per-book job statuses in Activity. Screenshots go to test-results/screenshots.
const DIR = 'test-results/screenshots';

async function openDoyle(page: Page) {
  await page.goto('/l/1/authors');
  await page.getByLabel('Filter authors').fill('Doyle Arthur Conan');
  await page.getByRole('button', { name: /Doyle Arthur Conan/ }).first().click();
  await expect(page.getByText('The Hound of the Baskervilles').first()).toBeVisible({ timeout: 15000 });
}

test.describe('desktop', () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test('send to my phone: QR code and short link from the details pane and the send dialog', async ({ page }) => {
    await openDoyle(page);
    await page.getByText('The Hound of the Baskervilles').first().click();
    await page.getByTestId('send-to-phone').click();
    const dialog = page.getByRole('dialog', { name: 'Send to my phone' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('img', { name: /QR code for http:\/\/.+\/h\/[A-Za-z0-9]{22}$/ })).toBeVisible();
    await expect(dialog.getByTestId('handoff-url')).toContainText('/h/');
    await expect(dialog.getByTestId('handoff-expires')).toContainText(/1[45]:\d\d/);
    await expect(dialog).toContainText('The Hound of the Baskervilles');
    // the link works without a session: the phone page and the EPUB download
    const url = (await dialog.getByTestId('handoff-url').getAttribute('title'))!;
    const res = await page.request.get(`${new URL(url).pathname}/file`);
    expect(res.status()).toBe(200);
    expect(res.headers()['content-type']).toBe('application/epub+zip');
    expect(res.headers()['content-disposition']).toMatch(/^attachment;/);
    await dialog.getByRole('button', { name: 'Close' }).last().click();

    // the send dialog offers the same for one selected book
    await page.getByRole('checkbox', { name: 'Select The Hound of the Baskervilles' }).check();
    await page.getByTestId('selection-send').click();
    const send = page.getByRole('dialog');
    await send.getByTestId('send-dialog-phone').click();
    await expect(send.getByTestId('handoff-panel').getByRole('img', { name: /QR code/ })).toBeVisible();
    await send.getByTestId('handoff-panel').scrollIntoViewIfNeeded();
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${DIR}/delivery-send-dialog-qr.png` });
  });

  test('send a whole series: per-book statuses, one e-mail, Kindle hint', async ({ page }) => {
    const series = await (await page.request.get('/api/v1/libraries/1/series')).json();
    const [sid] = (series.rows as [number, string, number][]).find((r) => r[2] >= 4 && r[2] <= 12)!;
    await page.goto(`/l/1/series/${sid}`);
    await page.getByTestId('send-series').click();
    const dialog = page.getByRole('dialog', { name: 'Send whole series' });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText('in reading order');
    await dialog.getByRole('button', { name: /^Kindle Send by email/ }).click();
    await expect(dialog).toContainText('up to 25 per e-mail');
    await dialog.getByRole('button', { name: 'Send series' }).click();
    await page.getByRole('button', { name: 'Activity' }).click();
    const job = page.getByTestId('job').first();
    await expect(job).toContainText('«');
    // states move through converting → … → accepted
    await expect(job.getByTestId('item-state').first()).toContainText(/Converting|Converted|Waiting|Handed/);
    await expect(job).toHaveAttribute('data-state', 'done', { timeout: 20000 });
    const states = await job.getByTestId('item-state').allTextContents();
    expect(states.length).toBeGreaterThanOrEqual(4);
    for (const s of states) expect(s).toMatch(/^Accepted by mail server · e-mail 1/);
    await expect(job).toContainText('250 2.0.0 Ok: queued as');
    await expect(job.getByTestId('kindle-hint')).toContainText("freelib@example.com is in Amazon's Approved Personal Document E-mail List");
    await expect(job.getByRole('link', { name: 'Amazon settings' })).toHaveAttribute('href', /amazon\.com/);
    await page.waitForTimeout(200);
    await page.screenshot({ path: `${DIR}/delivery-activity.png` });
  });

  test('selection bar: send the series of the selected books', async ({ page }) => {
    await openDoyle(page);
    const rows = await (await page.request.get('/api/v1/libraries/1/search?q=hound')).json();
    const hound = rows.books.find((b: { title: string }) => b.title === 'The Hound of the Baskervilles');
    test.skip(!hound?.series, 'demo book without a series');
    await page.getByRole('checkbox', { name: 'Select The Hound of the Baskervilles' }).check();
    await page.getByTestId('selection-send-series').click();
    await expect(page.getByRole('dialog', { name: 'Send whole series' })).toContainText(hound.series.name);
  });
});

test.describe('iPhone', () => {
  test.use({
    viewport: { width: 390, height: 844 },
    userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1',
    isMobile: true,
    hasTouch: true,
  });

  test('Open in Books is the primary action and downloads the EPUB', async ({ page }) => {
    const res = await (await page.request.get('/api/v1/libraries/1/search?q=hound')).json();
    const id = res.books[0].id;
    await page.goto(`/l/1/book/${id}`);
    const open = page.getByTestId('open-in-books');
    await expect(open).toBeVisible({ timeout: 15000 });
    await expect(open).toHaveText('Open in Books');
    await expect(page.getByTestId('send-caption')).toHaveText('Apple Books · EPUB');
    await expect(page.getByTestId('send-to-phone')).toHaveCount(0);
    const [download] = await Promise.all([page.waitForEvent('download'), open.click()]);
    expect(download.suggestedFilename()).toMatch(/\.epub$/);
    expect(download.url()).toMatch(/\/h\/[A-Za-z0-9]{22}\/file$/);
  });

  test('the phone page of a link', async ({ page }) => {
    const res = await (await page.request.get('/api/v1/libraries/1/search?q=hound')).json();
    const link = await (await page.request.post('/api/v1/handoff', {
      data: { library: 1, book: res.books[0].id }, headers: { 'Content-Type': 'application/json' },
    })).json();
    await page.goto(link.url);
    await expect(page.getByRole('link', { name: 'Open in Books' })).toBeVisible();
  });
});
