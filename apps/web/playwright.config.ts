import { defineConfig } from '@playwright/test';

export default defineConfig({
  expect: { timeout: 8_000 },
  forbidOnly: true,
  fullyParallel: false,
  outputDir: '../../artifacts/playwright/task-19-results',
  reporter: [['list']],
  testDir: './e2e',
  testMatch: '**/*.e2e.ts',
  timeout: 30_000,
  use: {
    baseURL: 'http://127.0.0.1:4173',
    browserName: 'chromium',
    channel: 'chrome',
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: {
    command:
      'corepack pnpm@10.28.0 --filter @agent-room/web exec vite --host 127.0.0.1 --port 4173 --strictPort',
    cwd: '../..',
    reuseExistingServer: true,
    timeout: 30_000,
    url: 'http://127.0.0.1:4173/connect',
  },
});
