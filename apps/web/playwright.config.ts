import { defineConfig } from '@playwright/test';

const capacityRun = process.env.AGENT_ROOM_CAPACITY_REPORT === '1';

export default defineConfig({
  expect: { timeout: 8_000 },
  forbidOnly: true,
  fullyParallel: false,
  outputDir: '../../artifacts/playwright/task-19-results',
  reporter: [['list']],
  testDir: './e2e',
  testMatch: '**/*.e2e.ts',
  timeout: 30_000,
  workers: process.env.CI ? 1 : 4,
  use: {
    baseURL: 'http://127.0.0.1:14173',
    browserName: 'chromium',
    ...(process.env.CI ? {} : { channel: 'chrome' as const }),
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
  },
  webServer: {
    command: capacityRun
      ? 'corepack pnpm@10.28.0 --filter @agent-room/web exec vite preview --host 127.0.0.1 --port 14173 --strictPort'
      : 'corepack pnpm@10.28.0 --filter @agent-room/web exec vite --host 127.0.0.1 --port 14173 --strictPort',
    cwd: '../..',
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
    url: 'http://127.0.0.1:14173/connect',
  },
});
