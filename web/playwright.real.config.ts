import { defineConfig, devices } from '@playwright/test';

// `pnpm test:real`: runs the specs under tests-real/ against the actual release server and a
// synthetic library (see tests-real/start-server.sh), instead of the mock backend that
// playwright.config.ts (`pnpm test`) uses. Requires `pnpm build` to have produced web/dist.
const PORT = Number(process.env.FREELIB_REAL_E2E_PORT ?? 8099);
const BASE_URL = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: './tests-real',
  timeout: 60_000,
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list']],
  globalSetup: './tests-real/global-setup.ts',
  use: {
    baseURL: BASE_URL,
    trace: 'retain-on-failure',
  },
  webServer: {
    command: 'bash ./tests-real/start-server.sh',
    url: `${BASE_URL}/api/v1/session`,
    reuseExistingServer: !process.env.CI,
    timeout: 10 * 60_000,
    env: { FREELIB_REAL_E2E_PORT: String(PORT) },
    stdout: 'pipe',
    stderr: 'pipe',
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
});
