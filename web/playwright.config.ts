import { defineConfig, devices } from '@playwright/test';

// PW_PORT: another port when several checkouts run their suites at once (a server already
// listening on the port is reused, whichever checkout it serves).
const PORT = Number(process.env.PW_PORT ?? 5183);

export default defineConfig({
  testDir: './tests',
  timeout: 30000,
  fullyParallel: true,
  retries: 0,
  reporter: [['list']],
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: 'retain-on-failure',
  },
  webServer: {
    command: `pnpm exec vite --port ${PORT} --strictPort`,
    port: PORT,
    reuseExistingServer: true,
    timeout: 30000,
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
  ],
});
