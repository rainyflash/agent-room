import { randomUUID } from 'node:crypto';

import { expect, test, type Browser, type BrowserContext, type Page } from '@playwright/test';

import { connectLiveSession } from '../e2e-live/support/live-session';
import type {
  VerticalSecuritySample,
  VerticalSecurityWindow,
} from '../src/test/vertical-security-driver';

const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const applicationOrigin = 'https://app.agent-room.localhost:18443';

test('真实 Synapse 完成首次信任、双设备 SAS 与新设备恢复', async ({ browser, page }) => {
  test.setTimeout(300_000);
  test.skip(username === undefined || password === undefined, '缺少隔离验收账户。');

  const recoveryPassphrase = `Agent Room task 27 ${randomUUID()}`;
  const additionalContexts: BrowserContext[] = [];
  try {
    await connectLiveSession(page, username ?? '', password ?? '');
    await openSecurity(page);
    await establishEncryptedIdentity(page);
    await setupRecovery(page, recoveryPassphrase);
    const recoverySample = await createRecoverySample(page);

    const second = await openDevice(browser, additionalContexts);
    await connectLiveSession(second, username ?? '', password ?? '');
    await openSecurity(second);
    await verifySecondDevice(page, second);

    const recovered = await openDevice(browser, additionalContexts);
    await connectLiveSession(recovered, username ?? '', password ?? '');
    await openSecurity(recovered);
    await recoverNewDevice(recovered, recoveryPassphrase);
    await decryptRecoverySample(recovered, recoverySample);
  } finally {
    await Promise.all(
      additionalContexts.map(async (context) => {
        await context.close();
      }),
    );
  }
});

async function openDevice(browser: Browser, contexts: BrowserContext[]): Promise<Page> {
  const context = await browser.newContext({
    baseURL: applicationOrigin,
    ignoreHTTPSErrors: true,
    locale: 'en-US',
  });
  contexts.push(context);
  return await context.newPage();
}

async function openSecurity(page: Page): Promise<void> {
  await page.goto('/settings/security');
  await expect(page.getByRole('heading', { level: 1 })).toContainText(/Security|安全中心/u, {
    timeout: 40_000,
  });
  await expect(page.locator('.security-account-line')).toBeVisible({ timeout: 40_000 });
}

async function establishEncryptedIdentity(page: Page): Promise<void> {
  const establish = page.getByRole('button', {
    name: /Establish encrypted identity|建立加密身份/u,
  });
  await expect(establish).toBeVisible({ timeout: 40_000 });
  await establish.click();
  await expect(currentDevice(page)).toContainText(/Verified|已验证/u, { timeout: 60_000 });
}

async function setupRecovery(page: Page, passphrase: string): Promise<void> {
  await page.getByRole('button', { name: /Set up recovery|设置恢复/u }).click();
  await page.getByLabel(/Recovery passphrase|恢复口令/u).fill(passphrase);
  await page.getByLabel(/Confirm passphrase|确认恢复口令/u).fill(passphrase);
  await page.getByRole('button', { name: /Create recovery|创建恢复/u }).click();
  await expect(page.getByText(/Save this recovery key now|立即保存这个恢复密钥/u)).toBeVisible({
    timeout: 90_000,
  });
  await expect(page.locator('.security-recovery-key output')).not.toBeEmpty();
  await page.getByRole('button', { name: /I saved the recovery key|我已保存恢复密钥/u }).click();
  await expect(page.locator('.security-recovery__status')).toContainText(
    /Recovery is ready|恢复能力已就绪/u,
    { timeout: 60_000 },
  );
}

async function verifySecondDevice(trusted: Page, candidate: Page): Promise<void> {
  await expect(currentDevice(candidate)).toContainText(/Unverified|未验证/u, { timeout: 60_000 });
  await candidate
    .getByRole('button', { name: /^Verify$|^验证$/u })
    .first()
    .click();

  const incoming = trusted.getByRole('dialog', {
    name: /Verify another signed-in device|验证另一台已登录设备/u,
  });
  await expect(incoming).toBeVisible({ timeout: 60_000 });
  await incoming.getByRole('button', { name: /Review codes|核对代码/u }).click();

  const trustedDialog = trusted.getByRole('dialog', {
    name: /Verify a Matrix device|验证 Matrix 设备/u,
  });
  const candidateDialog = candidate.getByRole('dialog', {
    name: /Verify a Matrix device|验证 Matrix 设备/u,
  });
  const trustedEmoji = trustedDialog.locator('.security-sas-emojis li');
  const candidateEmoji = candidateDialog.locator('.security-sas-emojis li');
  await expect(trustedEmoji).toHaveCount(7, { timeout: 60_000 });
  await expect(candidateEmoji).toHaveCount(7, { timeout: 60_000 });
  expect(await trustedEmoji.allTextContents()).toEqual(await candidateEmoji.allTextContents());

  await candidateDialog.getByRole('button', { name: /They match|完全一致/u }).click();
  await trustedDialog.getByRole('button', { name: /They match|完全一致/u }).click();
  await expect(candidateDialog).toContainText(/Device verified|设备验证完成/u, {
    timeout: 60_000,
  });
  await expect(trustedDialog).toContainText(/Device verified|设备验证完成/u, {
    timeout: 60_000,
  });
  await expect(currentDevice(candidate)).toContainText(/Verified|已验证/u, { timeout: 60_000 });
}

async function recoverNewDevice(page: Page, passphrase: string): Promise<void> {
  await expect(currentDevice(page)).toContainText(/Unverified|未验证/u, { timeout: 60_000 });
  await page.getByRole('button', { name: /Recover this device|恢复当前设备/u }).click();
  const recoveryCredential = page.getByLabel(/Passphrase or recovery key|恢复口令或恢复密钥/u);
  await expect(recoveryCredential).toBeVisible({ timeout: 10_000 });
  await recoveryCredential.fill(passphrase);
  await page.getByRole('button', { name: /Recover history|恢复历史/u }).click();
  await expect(page.locator('.security-recovery__complete')).toContainText(
    /Recovered [1-9]\d* of [1-9]\d* encrypted room keys|已恢复 [1-9]\d* \/ [1-9]\d* 个加密房间密钥/u,
    { timeout: 90_000 },
  );
  await expect(currentDevice(page)).toContainText(/Verified|已验证/u, { timeout: 60_000 });
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

async function decryptRecoverySample(page: Page, sample: VerticalSecuritySample): Promise<void> {
  await waitForVerticalSecurityDriver(page);
  await page.evaluate(async (candidate) => {
    const driver = (window as VerticalSecurityWindow).__agentRoomVerticalSecurityDriver;
    if (driver === undefined) {
      throw new Error('纵向安全驱动没有安装。');
    }
    await driver.decryptRecoverySample(candidate);
  }, sample);
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

function currentDevice(page: Page) {
  return page.locator('.security-devices__list > li.is-current');
}
