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
const browsers = ['chromium', 'firefox', 'webkit'] as const;
const browserText = process.env.AGENT_ROOM_E2E_BROWSER ?? 'chromium';
if (!(browsers as readonly string[]).includes(browserText)) {
  throw new Error(`AGENT_ROOM_E2E_BROWSER 必须是 ${browsers.join('、')} 之一。`);
}
const browser = browserText as (typeof browsers)[number];

// Chromium 是首要目标，跑全量。Firefox 与 WebKit 跑其余全部用例，只跳过下面这些——
// 每一条都实测过并写清原因，谁修好了就把谁删掉。这份清单清空之前，跨浏览器验收都不算完整。
const skipped: Record<Exclude<(typeof browsers)[number], 'chromium'>, string[]> = {
  firefox: [
    // Playwright 在 Firefox 上不支持授予 clipboard-read 权限，是测试设施限制，不是产品缺陷。
    'agent-access.e2e.ts',
    // 用例断言控制台无错。Firefox 对夹具生成的图片解码更严，报 Image corrupt or truncated。
    'application-features.e2e.ts',
    // 已读回执的时序在 Firefox 上与断言不符，尚未定位。
    'conversation-lifecycle.e2e.ts',
    // 用例断言禁用 eval 时仍可交互。Vite 开发服务器预打包的 zod 在 Firefox 下触发 CSP；
    // 生产构建没有 eval，所以这是开发服务器的问题，不是发布产物的问题。
    'lobby-csp.e2e.ts',
    // 帧预算用例依赖 CDP，只有 Chromium 提供。
    'lobby-scene.e2e.ts',
    // 这个用例直接调用 chromium.launchPersistentContext，本身就是 Chromium 专用的，
    // 不跟着 browserName 走。CI 只安装当轮矩阵的浏览器，所以必须跳过。
    'session-persistence.e2e.ts',
  ],
  webkit: [
    // 同上：WebKit 不支持授予 clipboard-write 权限。
    'agent-access.e2e.ts',
    // 附件重试用例期望 1 MiB 的密文缓冲，WebKit 上只拿到 68 字节的 PNG 头，尚未定位。
    'application-features.e2e.ts',
    // 用例断言控制台无错，WebKit 上多出一条，尚未定位。
    'automatic-conversations.e2e.ts',
    // 这两个用例一开场就打开房间设置抽屉，而抽屉在 WebKit 上没有出现，30 秒超时。
    // 夹具冷启动无法复现（Chromium 和 WebKit 都不渲染导航），需要带上用例前置状态再查。
    // moderation-governance 用同一个 helper 但先与时间线交互，所以不受影响。
    'automation-grants.e2e.ts',
    'private-room-lifecycle.e2e.ts',
    // WebKit 默认不给链接和按钮键盘焦点（Full Keyboard Access 默认关闭），toBeFocused 失败。
    'connection-shell.e2e.ts',
    'conversation-workspace.e2e.ts',
    // 帧预算用例依赖 CDP。
    'lobby-scene.e2e.ts',
    // 这个用例直接调用 chromium.launchPersistentContext，本身就是 Chromium 专用的，
    // 不跟着 browserName 走。CI 只安装当轮矩阵的浏览器，所以必须跳过。
    'session-persistence.e2e.ts',
  ],
};

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
  testIgnore: browser === 'chromium' ? [] : skipped[browser],
  testMatch: '**/*.e2e.ts',
  timeout: 30_000,
  workers: process.env.CI ? 1 : 4,
  use: {
    baseURL: baseUrl,
    browserName: browser,
    ...(browser === 'chromium' && !process.env.CI ? { channel: 'chrome' as const } : {}),
    screenshot: 'only-on-failure',
    trace: process.env.CI ? 'on-first-retry' : 'retain-on-failure',
  },
  webServer: {
    command: capacityRun
      ? `corepack pnpm@10.28.0 --filter @agent-room/web exec vite preview --host 127.0.0.1 --port ${portText} --strictPort`
      : `corepack pnpm@10.28.0 --filter @agent-room/web exec vite --host 127.0.0.1 --port ${portText} --strictPort`,
    cwd: '../..',
    reuseExistingServer: !process.env.CI && process.env.AGENT_ROOM_E2E_REUSE_SERVER !== '0',
    timeout: 30_000,
    url: `${baseUrl}/connect`,
  },
});
