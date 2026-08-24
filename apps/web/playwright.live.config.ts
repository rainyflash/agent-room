import { defineConfig } from '@playwright/test';

export default defineConfig({
  expect: { timeout: 20_000 },
  forbidOnly: true,
  fullyParallel: false,
  outputDir: '../../artifacts/playwright/task-19-live-results',
  reporter: [['list']],
  testDir: './e2e-live',
  testMatch: '**/*.e2e.ts',
  timeout: 120_000,
  workers: 1,
  use: {
    baseURL: 'https://app.agent-room.localhost:18443',
    browserName: 'chromium',
    ...(process.env.CI ? {} : { channel: 'chrome' as const }),
    ignoreHTTPSErrors: true,
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: [
    {
      command: 'python tools/control-plane.py run',
      cwd: '../..',
      reuseExistingServer: !process.env.CI,
      timeout: 120_000,
      url: 'http://127.0.0.1:8090/health/live',
    },
    {
      command:
        'corepack pnpm@10.28.0 --filter @agent-room/web exec vite --host 0.0.0.0 --port 5173 --strictPort',
      cwd: '../..',
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
      url: 'http://127.0.0.1:5173/connect',
    },
  ],
});
