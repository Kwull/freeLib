import { test, expect } from '@playwright/test';
import { pickFb2Book } from './helpers';

// The server sends a strict Content-Security-Policy (docs/web/API.md). The reader renders EPUB
// sections from blob: URLs in a same-origin iframe (sandbox keeps allow-scripts for WebKit), so
// book scripts must be blocked by the inherited policy, and the reader must still work under it.
test('reader works under the CSP and book scripts cannot run', async ({ page, request }) => {
  const errors: string[] = [];
  page.on('console', (msg) => { if (msg.type() === 'error') errors.push(msg.text()); });
  page.on('pageerror', (err) => errors.push(String(err)));

  const res = await request.get('/');
  const csp = res.headers()['content-security-policy'] ?? '';
  expect(csp).toContain("script-src 'self'");
  expect(csp).toContain("object-src 'none'");

  const book = await pickFb2Book(request, 1);
  const file = await request.get(`/api/v1/libraries/1/books/${book.id}/file?format=original&inline=1`);
  expect(file.headers()['content-disposition']).toMatch(/^attachment;/);
  expect(file.headers()['content-security-policy']).toBe('sandbox');

  await page.goto(`/l/1/read/${book.id}`);
  const progress = page.getByLabel('Progress');
  await expect(progress).toBeVisible({ timeout: 15000 });
  await page.getByRole('button', { name: 'Next page' }).click();
  await page.waitForTimeout(300);

  // Same setup as foliate's paginator: a blob document in a same-origin iframe with scripts
  // allowed by the sandbox. The inherited CSP must still block inline scripts and handlers.
  const pwned = await page.evaluate(async () => {
    const w = window as unknown as { __pwned?: string };
    const html = '<!doctype html><body><script>parent.__pwned = "script"</script>'
      + '<img src="data:," onerror="parent.__pwned = \'handler\'"></body>';
    const url = URL.createObjectURL(new Blob([html], { type: 'text/html' }));
    const f = document.createElement('iframe');
    f.setAttribute('sandbox', 'allow-same-origin allow-scripts');
    const loaded = new Promise((r) => f.addEventListener('load', r, { once: true }));
    f.src = url;
    document.body.append(f);
    await loaded;
    await new Promise((r) => setTimeout(r, 300));
    f.remove();
    return w.__pwned ?? null;
  });
  expect(pwned).toBeNull();
  // … and it was the policy that stopped them
  await expect.poll(() => errors.some((e) => /Content Security Policy/.test(e))).toBe(true);

  // only the violations provoked above; the reader itself must not trip the policy
  const unexpected = errors.filter((e) => !/Content Security Policy/.test(e) || !/inline|script-src-elem|script-src-attr/.test(e));
  expect(unexpected, unexpected.join('\n')).toEqual([]);
});
