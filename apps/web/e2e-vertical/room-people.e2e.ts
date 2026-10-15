import { access, readFile, writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import { expect, test, type Browser, type BrowserContext, type Page } from '@playwright/test';
import { z } from 'zod';

import {
  collectUnhandledFailures,
  connectLiveSession,
  type LiveSessionCredentials,
} from '../e2e-live/support/live-session';

const applicationOrigin = 'https://app.agent-room.localhost:18443';
const guestUsername = process.env.AGENT_ROOM_E2E_USERNAME;
const guestPassword = process.env.AGENT_ROOM_E2E_PASSWORD;
const ownerUsername = process.env.AGENT_ROOM_VERTICAL_ROOM_PEOPLE_OWNER_USERNAME;
const ownerPassword = process.env.AGENT_ROOM_VERTICAL_ROOM_PEOPLE_OWNER_PASSWORD;
const roomName = process.env.AGENT_ROOM_VERTICAL_ROOM_PEOPLE_NAME;
const joinedPath = process.env.AGENT_ROOM_VERTICAL_ROOM_PEOPLE_JOINED;
const spokePath = process.env.AGENT_ROOM_VERTICAL_ROOM_PEOPLE_SPOKE;
const checkedPath = process.env.AGENT_ROOM_VERTICAL_ROOM_PEOPLE_CHECKED;

const inviteUrlPattern = /https:\/\/app\.agent-room\.localhost:18443\/join#[0-9a-hjkmnp-tv-z]{26}/u;
const roomPathPattern = /\/lobby\/([0-9a-f-]{36})\/instance\//u;
const spokeSchema = z.object({ agentName: z.string().min(1), text: z.string().min(1) }).strict();

async function exists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

async function openIsolatedPage(browser: Browser, contexts: BrowserContext[]): Promise<Page> {
  const context = await browser.newContext({
    baseURL: applicationOrigin,
    ignoreHTTPSErrors: true,
    locale: 'en-US',
  });
  contexts.push(context);
  // “生成链接并复制”会写剪贴板；无头浏览器默认不给，复制失败也不算错，这里给上免得多一条报错。
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], {
    origin: applicationOrigin,
  });
  return await context.newPage();
}

// 房主在真实网页上建私人房间，从“邀请人”里生成链接（specs/room-people/design.md 第 6 步）。
async function makeRoomAndLink(owner: Page, name: string): Promise<string> {
  await owner.getByRole('button', { name: 'New room', exact: true }).click();
  const created = owner.getByRole('dialog', { name: 'New room' });
  await created.getByLabel('Name', { exact: true }).fill(name);
  await created.getByRole('button', { name: 'Create and enter', exact: true }).click();
  await expect(owner).toHaveURL(roomPathPattern, { timeout: 90_000 });

  await owner.getByRole('button', { name: 'Invite people', exact: true }).click();
  const dialog = owner.getByRole('dialog', { name: 'Invite people' });
  await expect(dialog).toContainText('There is no link yet.', { timeout: 30_000 });
  await dialog.getByRole('button', { name: 'Make link and copy' }).click();
  await expect(dialog).toContainText(inviteUrlPattern, { timeout: 30_000 });
  const link = inviteUrlPattern.exec(await dialog.innerText())?.[0];
  if (link === undefined) throw new Error('“邀请人”里没有看到链接。');
  await owner.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  return link;
}

// 没登录的人打开链接：先去登录，回来以后还在这一页；秘密在登录跳转之前就从地址栏去掉了。
async function openLinkSignedOut(
  guest: Page,
  link: string,
  credentials: LiveSessionCredentials,
): Promise<void> {
  await guest.goto(link);
  await expect(guest).toHaveURL(/\/connect\?returnTo=(?:%2F|\/)join$/u, { timeout: 40_000 });
  await guest.getByRole('button', { name: /Sign in to Agent Room|登录 Agent Room/u }).click();
  await expect(guest).toHaveURL(/\/realms\/agent-room\/protocol\/openid-connect\/auth/u);
  expect(guest.url()).not.toContain(link.slice(link.indexOf('#') + 1));
  await guest.locator('input[name="username"]').fill(credentials.username);
  await guest.locator('input[name="password"]').fill(credentials.password);
  await guest.locator('input[type="submit"], button[type="submit"]').click();

  // 回到 `/join` 以后还要登录聊天服务器：再跳去它那边一趟。登录过聊天服务器的人会先看到一页
  // “Continue”，点了才回来。所以等的是邀请的标题，不是地址；中途看到“Continue”就点。
  const consent = guest.getByRole('link', { name: /^Continue$/u });
  const invitation = guest.getByRole('heading', { name: /invited you to/u });
  await expect
    .poll(
      async () => {
        if (await consent.isVisible()) {
          // 点完页面就跳走，点的这一下报错也不要紧，下一轮再看。
          await consent.click().catch(() => undefined);
          return 'consent';
        }
        if (await invitation.isVisible()) return 'ready';
        return `pending at ${new URL(guest.url()).pathname}`;
      },
      { timeout: 90_000 },
    )
    .toBe('ready');
  await expect(guest).toHaveURL(/\/join$/u);
}

async function openConversation(page: Page): Promise<void> {
  const room = new URL(page.url());
  room.searchParams.set('view', 'conversation');
  await page.goto(`${room.pathname}${room.search}`);
}

async function expectOwnerLabel(page: Page, agentName: string, label: string): Promise<void> {
  const room = new URL(page.url());
  room.searchParams.delete('view');
  await page.goto(`${room.pathname}${room.search}`);
  // 服务器只回答此刻在房间里的 Agent 是谁的；刚进来的过 30 秒再问一次。
  await expect(
    page.getByRole('option', { name: new RegExp(`^${agentName}, ${label}, `, 'u') }),
  ).toHaveCount(1, { timeout: 120_000 });
  await page.getByRole('button', { name: 'Find someone', exact: true }).click();
  const members = page.getByRole('dialog', { name: 'Agents in this room', exact: true });
  await expect(
    members.getByRole('button', { name: new RegExp(`^${agentName}`, 'u') }),
  ).toContainText(label);
  await page.keyboard.press('Escape');
}

test('房主生成邀请链接，第二个人登录后加入，他的 Agent 进来说话，两边都看得出是谁的', async ({
  browser,
}) => {
  test.skip(
    guestUsername === undefined ||
      guestPassword === undefined ||
      ownerUsername === undefined ||
      ownerPassword === undefined ||
      roomName === undefined ||
      joinedPath === undefined ||
      spokePath === undefined ||
      checkedPath === undefined,
    '缺少房间里的人纵向验收变量。',
  );
  test.setTimeout(900_000);
  for (const path of [joinedPath, spokePath, checkedPath])
    expect(isAbsolute(path ?? '')).toBe(true);
  const name = roomName ?? '';

  const contexts: BrowserContext[] = [];
  try {
    const owner = await openIsolatedPage(browser, contexts);
    const guest = await openIsolatedPage(browser, contexts);
    const failures = [...collectUnhandledFailures(owner), ...collectUnhandledFailures(guest)];

    await connectLiveSession(owner, {
      expectedDisplayName: 'Local Collaborator',
      password: ownerPassword ?? '',
      username: ownerUsername ?? '',
    });
    const link = await makeRoomAndLink(owner, name);
    const catalogId = roomPathPattern.exec(owner.url())?.[1] ?? '';

    await openLinkSignedOut(guest, link, {
      expectedDisplayName: 'Local Developer',
      password: guestPassword ?? '',
      username: guestUsername ?? '',
    });
    await expect(
      guest.getByRole('heading', { name: `Local Collaborator invited you to “${name}”` }),
    ).toBeVisible({ timeout: 60_000 });
    await expect(guest.getByText('agents you bring in can talk too')).toBeVisible();
    await guest.getByRole('button', { name: 'Join', exact: true }).click();
    await expect(guest).toHaveURL(new RegExp(`/lobby/${catalogId}/instance/`, 'u'), {
      timeout: 90_000,
    });
    await writeFile(joinedPath ?? '', `${JSON.stringify({ catalogId })}\n`, 'utf8');

    // 纵向验收让他的本机 Agent 按房间名进来说一句，说完写下 spoke 文件。
    await expect
      .poll(async () => await exists(spokePath ?? ''), { intervals: [1_000], timeout: 420_000 })
      .toBe(true);
    const spoke = spokeSchema.parse(JSON.parse(await readFile(spokePath ?? '', 'utf8')));

    for (const page of [owner, guest]) {
      await openConversation(page);
      await expect(
        page.getByRole('region', { name: 'Conversation', exact: true }).getByRole('log'),
      ).toContainText(spoke.text, { timeout: 120_000 });
    }
    await expectOwnerLabel(owner, spoke.agentName, 'Local Developer’s agent');
    await expectOwnerLabel(guest, spoke.agentName, 'Your agent');
    expect(failures).toEqual([]);
    await writeFile(checkedPath ?? '', `${JSON.stringify({ checked: true })}\n`, 'utf8');
  } finally {
    await Promise.all(
      contexts.map(async (context) => {
        await context.close();
      }),
    );
  }
});
