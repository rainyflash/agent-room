import { defineConfig } from '@playwright/test';

// 服务与凭据由 tools/private_chat.py 隔离编排，禁止接入已安装的 Bridge。
export default defineConfig({
  expect: { timeout: 30_000 },
  forbidOnly: true,
  fullyParallel: false,
  outputDir: '../../artifacts/private-chat/browser-results',
  reporter: [['list']],
  testDir: './e2e-live',
  testMatch: 'private-chat.integration.ts',
  timeout: 300_000,
  workers: 1,
  use: {
    baseURL: 'https://app.agent-room.localhost:18443',
    browserName: 'chromium',
    ...(process.env.CI ? {} : { channel: 'chrome' }),
    ignoreHTTPSErrors: true,
    locale: 'en-US',
    viewport: { width: 1440, height: 900 },
    trace: 'off',
  },
});
