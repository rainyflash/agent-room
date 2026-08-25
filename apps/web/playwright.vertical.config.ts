import { defineConfig } from '@playwright/test';

export default defineConfig({
  expect: { timeout: 20_000 },
  forbidOnly: true,
  fullyParallel: false,
  outputDir: '../../artifacts/browser/task-24/results',
  reporter: [['list']],
  testDir: './e2e-vertical',
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
});
