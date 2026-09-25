// Waits for the synthetic library's autoimport (started by `start-server.sh`) to finish before
// any real-server spec runs. Playwright's `webServer.url` only proves the HTTP server itself is
// up (`GET /session` never fails); the import that `FREELIB_AUTOIMPORT` kicked off runs in the
// background and can take a while for a 20k-book synthetic library.
import { request } from '@playwright/test';

export default async function globalSetup(): Promise<void> {
  const base = process.env.FREELIB_REAL_E2E_BASE_URL ?? `http://127.0.0.1:${process.env.FREELIB_REAL_E2E_PORT ?? '8099'}`;
  const ctx = await request.newContext({ baseURL: base });
  const deadline = Date.now() + 5 * 60_000;
  let last = '';
  try {
    while (Date.now() < deadline) {
      const res = await ctx.get('/api/v1/libraries').catch(() => null);
      if (res && res.ok()) {
        const libs = (await res.json()) as { status: { state: string }; bookCount: number }[];
        last = JSON.stringify(libs.map((l) => [l.status.state, l.bookCount]));
        if (libs.length > 0 && libs.every((l) => l.status.state === 'idle') && libs.some((l) => l.bookCount > 0)) {
          return;
        }
      }
      await new Promise((r) => setTimeout(r, 1000));
    }
  } finally {
    await ctx.dispose();
  }
  throw new Error(`real server: library import did not finish in time (last seen: ${last})`);
}
