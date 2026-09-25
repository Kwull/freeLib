import { test, expect } from '@playwright/test';
import { pickBiggestGenre } from './helpers';

test('library ratings from the INPX: sort a genre on the server', async ({ page, request }) => {
  const g = await pickBiggestGenre(request);
  const res = await request.get(`/api/v1/libraries/1/books?genre=${g.id}&sort=lib&limit=50`);
  const body = await res.json();
  const lr: number[] = body.books.map((b: { libRating: number }) => b.libRating);
  expect(lr[0]).toBe(5);
  for (let i = 1; i < lr.length; i++) expect(lr[i - 1]).toBeGreaterThanOrEqual(lr[i]);

  await page.goto(`/l/1/genres/${g.id}`);
  await expect(page.locator('.scroll .brow').first()).toBeVisible({ timeout: 15000 });
  const req = page.waitForResponse((r) => r.url().includes('sort=lib') && r.url().includes('minLib=4'));
  await page.getByTestId('books-sort').selectOption('libRating');
  await page.getByRole('button', { name: /Filter/ }).click();
  await page.getByTestId('filter-min-lib').selectOption('4');
  const r = await req;
  expect(r.status()).toBe(200);
  const j = await r.json();
  expect(j.books.every((b: { libRating: number }) => b.libRating >= 4)).toBe(true);
});

test('a token made in Settings works for MCP', async ({ page, request }) => {
  await page.goto('/settings/account');
  const box = page.getByTestId('api-tokens');
  await box.getByTestId('token-name').fill('e2e');
  await box.getByTestId('token-create').click();
  const secret = (await box.getByTestId('token-secret').locator('code').textContent())!.trim();
  expect(secret).toMatch(/^fl_/);
  const url = (await box.getByTestId('mcp-url').textContent())!.trim();
  expect(url).toMatch(/\/mcp$/);

  const call = (method: string, params: unknown) =>
    request.post('/mcp', {
      headers: {
        Authorization: `Bearer ${secret}`,
        Accept: 'application/json, text/event-stream',
        'Content-Type': 'application/json',
        'MCP-Protocol-Version': '2025-06-18',
      },
      data: { jsonrpc: '2.0', id: 1, method, params },
    });
  const init = await call('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'e2e', version: '1' } });
  expect(init.status()).toBe(200);
  expect((await init.json()).result.serverInfo.name).toBe('freelib');
  const list = await (await call('tools/list', {})).json();
  const names = list.result.tools.map((t: { name: string }) => t.name);
  expect(names).toContain('search_books');
  expect(names).not.toContain('send_books'); // read-only token
  const genres = await (await call('tools/call', { name: 'list_genres', arguments: {} })).json();
  expect(genres.result.structuredContent.genres.length).toBeGreaterThan(0);
  // without the token
  const anon = await request.post('/mcp', { headers: { 'Content-Type': 'application/json' }, data: {} });
  expect(anon.status()).toBe(401);

  await page.reload();
  await expect(page.getByTestId('token-audit')).toContainText('list_genres');
});
