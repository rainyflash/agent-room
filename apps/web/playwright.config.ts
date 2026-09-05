import { defineConfig } from '@playwright/test';

const capacityRun = process.env.AGENT_ROOM_CAPACITY_REPORT === '1';
const portText = process.env.AGENT_ROOM_E2E_PORT ?? '14173';
if (!/^\d+$/u.test(portText)) {
  throw new Error('AGENT_ROOM_E2E_PORT 必须是十进制端口。');
}
const e2ePort = Number.parseInt(portText, 10);
if (e2ePort < 1 || e2ePort > 65_535) {
  throw new Error('AGENT_ROOM_E2E_PORT 必须位于 1..65535。');
}
const baseUrl = `http://127.0.0.1:${portText}`;

export default defineConfig({
  expect: { timeout: 8_000 },
  // 重试仅用于收集诊断；首轮失败即使重试通过，也不能放行 CI。
  failOnFlakyTests: true,
  forbidOnly: true,
  fullyParallel: false,
  outputDir: '../../artifacts/playwright/task-19-results',
  reporter: [['list']],
  retries: process.env.CI ? 1 : 0,
  testDir: './e2e',
  testMatch: '**/*.e2e.ts',
  timeout: 30_000,
  workers: process.env.CI ? 1 : 4,
  use: {
    baseURL: baseUrl,
    browserName: 'chromium',
    ...(process.env.CI ? {} : { channel: 'chrome' as const }),
    screenshot: 'only-on-failure',
    trace: process.env.CI ? 'on-first-retry' : 'retain-on-failure',
  },
  webServer: {
    command: capacityRun
      ? `corepack pnpm@10.28.0 --filter @agent-room/web exec vite preview --host 127.0.0.1 --port ${portText} --strictPort`
      : `corepack pnpm@10.28.0 --filter @agent-room/web exec vite --host 127.0.0.1 --port ${portText} --strictPort`,
    cwd: '../..',
    reuseExistingServer: !process.env.CI,
    timeout: 30_000,
    url: `${baseUrl}/connect`,
  },
});
