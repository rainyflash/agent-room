import { expect, test, type Browser, type BrowserContext, type Page } from '@playwright/test';

import { connectLiveSession } from '../e2e-live/support/live-session';
import type {
  VerticalSecuritySample,
  VerticalSecurityWindow,
} from '../src/test/vertical-security-driver';

const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const applicationOrigin = 'https://app.agent-room.localhost:18443';

// 设备自动签名（ADR 0011）：不输恢复密钥、不核对设备，登录就签好；新设备自动找回加密历史。
test('真实 Synapse 上登录即签好设备，新设备自动签上并读回加密历史', async ({ browser, page }) => {
  test.setTimeout(300_000);
  test.skip(username === undefined || password === undefined, '缺少隔离验收账户。');

  const additionalContexts: BrowserContext[] = [];
  try {
    await connectDeveloperSession(page);
    await expectDeviceReady(page);
    const sample = await createRecoverySample(page);

    const second = await openDevice(browser, additionalContexts);
    await connectDeveloperSession(second);
    await expectDeviceReady(second);
    await expectSampleReadable(second, sample);
  } finally {
    await Promise.all(
      additionalContexts.map(async (context) => {
        await context.close();
      }),
    );
  }
});

async function connectDeveloperSession(page: Page): Promise<string> {
  return await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });
}

async function openDevice(browser: Browser, contexts: BrowserContext[]): Promise<Page> {
  const context = await browser.newContext({
    baseURL: applicationOrigin,
    ignoreHTTPSErrors: true,
    locale: 'en-US',
  });
  contexts.push(context);
  return await context.newPage();
}

/** 安全页说这台设备已就绪，设备列表里它也标着已由你签名。 */
async function expectDeviceReady(page: Page): Promise<void> {
  await page.goto('/settings/security');
  await expect(page.getByText(/^(?:This device is ready|这台设备已就绪)$/u)).toBeVisible({
    timeout: 90_000,
  });
  await expect(page.locator('.security-devices__list > li.is-current')).toContainText(
    /Signed by you|已由你签名/u,
    { timeout: 60_000 },
  );
  await expect(page.getByText(/recovery key|恢复密钥/iu)).toHaveCount(0);
}

async function createRecoverySample(page: Page): Promise<VerticalSecuritySample> {
  await waitForVerticalSecurityDriver(page);
  return await page.evaluate(async () => {
    const driver = (window as VerticalSecurityWindow).__agentRoomVerticalSecurityDriver;
    if (driver === undefined) {
      throw new Error('纵向安全驱动没有安装。');
    }
    return await driver.createRecoverySample();
  });
}

/** 新设备在后台从服务器端备份找回房间密钥，找回之前解不开，所以轮询。 */
async function expectSampleReadable(page: Page, sample: VerticalSecuritySample): Promise<void> {
  await waitForVerticalSecurityDriver(page);
  await expect
    .poll(
      async () =>
        await page.evaluate(async (candidate) => {
          const driver = (window as VerticalSecurityWindow).__agentRoomVerticalSecurityDriver;
          if (driver === undefined) return false;
          try {
            await driver.decryptRecoverySample(candidate);
            return true;
          } catch {
            return false;
          }
        }, sample),
      { timeout: 90_000 },
    )
    .toBe(true);
}

async function waitForVerticalSecurityDriver(page: Page): Promise<void> {
  await expect
    .poll(
      async () =>
        await page.evaluate(
          () => (window as VerticalSecurityWindow).__agentRoomVerticalSecurityDriver !== undefined,
        ),
      { timeout: 20_000 },
    )
    .toBe(true);
}
