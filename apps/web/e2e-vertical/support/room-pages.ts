import { access } from 'node:fs/promises';

import {
  expect,
  type Browser,
  type BrowserContext,
  type Locator,
  type Page,
} from '@playwright/test';

// 公开房间那几轮共用的网页操作。
const applicationOrigin = 'https://app.agent-room.localhost:18443';

export const roomPathPattern = /\/lobby\/([0-9a-f-]{36})\/instance\//u;

export function inRoom(catalogId: string): RegExp {
  return new RegExp(`/lobby/${catalogId}/instance/`, 'u');
}

export async function openIsolatedPage(
  browser: Browser,
  contexts: BrowserContext[],
): Promise<Page> {
  const context = await browser.newContext({
    baseURL: applicationOrigin,
    ignoreHTTPSErrors: true,
    locale: 'en-US',
  });
  contexts.push(context);
  return await context.newPage();
}

// 从地址栏打开页面以后消息要重新连上，等这台设备好了再动手，加入、建房都要用到它。
export async function waitUntilDeviceReady(page: Page): Promise<void> {
  await page.goto('/settings/security');
  await expect(page.getByText('This device is ready', { exact: true })).toBeVisible({
    timeout: 60_000,
  });
}

export async function openRoomSettings(page: Page, section: string): Promise<Locator> {
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Room settings' });
  await dialog.getByRole('radio', { name: section, exact: true }).click();
  return dialog;
}

export async function closeRoomSettings(page: Page, settings: Locator): Promise<void> {
  await page.keyboard.press('Escape');
  await expect(settings).toBeHidden();
}

export async function openConversation(page: Page): Promise<void> {
  const room = new URL(page.url());
  room.searchParams.set('view', 'conversation');
  await page.goto(`${room.pathname}${room.search}`);
}

export function conversationLog(page: Page): Locator {
  return page.getByRole('region', { name: 'Conversation', exact: true }).getByRole('log');
}

export async function say(page: Page, text: string): Promise<void> {
  const conversation = page.getByRole('region', { name: 'Conversation', exact: true });
  const box = conversation.getByRole('textbox', { name: 'Message', exact: true });
  await expect(box).toBeEnabled({ timeout: 120_000 });
  await box.fill(text);
  await conversation.getByRole('button', { name: 'Send', exact: true }).click();
  // 刚改回私密时这台设备可能还没同步到加密状态：网页不发明文，说没发出去，过一会儿重试就好。
  const retry = page.getByRole('button', { name: 'Retry this message', exact: true });
  await expect
    .poll(
      async () => {
        if (await retry.isVisible()) await retry.click().catch(() => undefined);
        return await conversationLog(page).getByText(text).count();
      },
      { intervals: [3_000], timeout: 120_000 },
    )
    .toBeGreaterThan(0);
}

export async function expectSaid(page: Page, text: string): Promise<void> {
  await expect(conversationLog(page)).toContainText(text, { timeout: 120_000 });
}

// 乙从发现页找到房间，一点就进来了。
export async function joinFromExplore(
  member: Page,
  name: string,
  catalogId: string,
): Promise<void> {
  await member.getByRole('link', { name: 'Explore', exact: true }).click();
  await member
    .getByRole('searchbox', { name: 'Search room names, descriptions and owners' })
    .fill(name);
  await member.getByRole('button', { name: `Join · ${name}`, exact: true }).click();
  await expect(member).toHaveURL(inRoom(catalogId), { timeout: 90_000 });
}

export async function waitForStep(path: string): Promise<void> {
  await expect
    .poll(
      async () => {
        try {
          await access(path);
          return true;
        } catch {
          return false;
        }
      },
      { intervals: [1_000], timeout: 420_000 },
    )
    .toBe(true);
}
