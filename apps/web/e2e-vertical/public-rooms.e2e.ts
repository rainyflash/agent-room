import { writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import {
  expect,
  test,
  type APIRequestContext,
  type Browser,
  type BrowserContext,
  type Locator,
  type Page,
} from '@playwright/test';

import {
  apiOrigin,
  collectUnhandledFailures,
  connectLiveSession,
  submitIdentityForm,
} from '../e2e-live/support/live-session';

const applicationOrigin = 'https://app.agent-room.localhost:18443';
const ownerUsername = process.env.AGENT_ROOM_E2E_USERNAME;
const ownerPassword = process.env.AGENT_ROOM_E2E_PASSWORD;
const memberUsername = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_MEMBER_USERNAME;
const memberPassword = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_MEMBER_PASSWORD;
const visitorUsername = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_VISITOR_USERNAME;
const visitorPassword = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_VISITOR_PASSWORD;
const roomName = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_NAME;
const saidInPublic = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_SAID_PUBLIC;
const saidInPrivate = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_SAID_PRIVATE;
const resultPath = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_RESULT;

const roomPathPattern = /\/lobby\/([0-9a-f-]{36})\/instance\//u;
const shareCodePattern = /\/r\/([A-Za-z0-9]{6})/u;

function inRoom(catalogId: string): RegExp {
  return new RegExp(`/lobby/${catalogId}/instance/`, 'u');
}

function shareCodeIn(text: string): string {
  const code = shareCodePattern.exec(text)?.[1];
  if (code === undefined) throw new Error('没有看到分享链接。');
  return code;
}

// 分享链接在裸域上，本地的裸域是聊天服务器的域名、网关也不转 `/r/`。分享页由控制面出，这里直接走它。
function sharePage(code: string): string {
  return `${apiOrigin}/r/${code}`;
}

async function openIsolatedPage(browser: Browser, contexts: BrowserContext[]): Promise<Page> {
  const context = await browser.newContext({
    baseURL: applicationOrigin,
    ignoreHTTPSErrors: true,
    locale: 'en-US',
  });
  contexts.push(context);
  return await context.newPage();
}

// 从地址栏打开页面以后消息要重新连上，等这台设备好了再动手，加入、建房都要用到它。
async function waitUntilDeviceReady(page: Page): Promise<void> {
  await page.goto('/settings/security');
  await expect(page.getByText('This device is ready', { exact: true })).toBeVisible({
    timeout: 60_000,
  });
}

async function openRoomSettings(page: Page, section: string): Promise<Locator> {
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Room settings' });
  await dialog.getByRole('radio', { name: section, exact: true }).click();
  return dialog;
}

async function closeRoomSettings(page: Page, settings: Locator): Promise<void> {
  await page.keyboard.press('Escape');
  await expect(settings).toBeHidden();
}

async function createPublicRoom(owner: Page, name: string): Promise<string> {
  await owner.getByRole('link', { name: 'Rooms', exact: true }).click();
  await owner.getByRole('button', { name: 'New room', exact: true }).click();
  const dialog = owner.getByRole('dialog', { name: 'New room' });
  await dialog.getByLabel('Name', { exact: true }).fill(name);
  await dialog
    .getByRole('group', { name: 'Who can find this room' })
    .getByRole('radio', { name: /^Public/u })
    .check();
  await expect(
    dialog.getByRole('group', { name: 'How people join' }).getByRole('radio', {
      name: /^Join directly/u,
    }),
  ).toBeChecked();
  await dialog.getByRole('button', { name: 'Create and enter', exact: true }).click();
  await expect(owner).toHaveURL(roomPathPattern, { timeout: 90_000 });
  const catalogId = roomPathPattern.exec(owner.url())?.[1];
  if (catalogId === undefined) throw new Error('建好以后不在房间里。');
  return catalogId;
}

async function openConversation(page: Page): Promise<void> {
  const room = new URL(page.url());
  room.searchParams.set('view', 'conversation');
  await page.goto(`${room.pathname}${room.search}`);
}

function conversationLog(page: Page): Locator {
  return page.getByRole('region', { name: 'Conversation', exact: true }).getByRole('log');
}

async function say(page: Page, text: string): Promise<void> {
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

async function expectSaid(page: Page, text: string): Promise<void> {
  await expect(conversationLog(page)).toContainText(text, { timeout: 120_000 });
}

// 乙从发现页找到房间，一点就进来了。
async function joinFromExplore(member: Page, name: string, catalogId: string): Promise<void> {
  await member.getByRole('link', { name: 'Explore', exact: true }).click();
  await member
    .getByRole('searchbox', { name: 'Search room names, descriptions and owners' })
    .fill(name);
  await member.getByRole('button', { name: `Join · ${name}`, exact: true }).click();
  await expect(member).toHaveURL(inRoom(catalogId), { timeout: 90_000 });
}

async function readShareCode(member: Page): Promise<string> {
  await member.getByRole('button', { name: 'Share', exact: true }).click();
  const dialog = member.getByRole('dialog', { name: 'Share this room' });
  const code = shareCodeIn(await dialog.getByLabel('Link', { exact: true }).inputValue());
  await member.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  return code;
}

// 不带登录的请求也看得到房间名，发到别处时预览上写的是它。
async function expectSharePage(api: APIRequestContext, code: string, name: string): Promise<void> {
  const page = await api.get(sharePage(code), { headers: { 'Accept-Language': 'en' } });
  expect(page.status()).toBe(200);
  const html = await page.text();
  expect(html).toContain(`<h1>${name}</h1>`);
  expect(html).toContain(`<meta property="og:title" content="${name} · Agent Room">`);
}

async function expectShareGone(api: APIRequestContext, code: string, name: string): Promise<void> {
  const page = await api.get(sharePage(code), { headers: { 'Accept-Language': 'en' } });
  expect(page.status()).toBe(404);
  expect(await page.text()).not.toContain(name);
}

// 没登录的人打开分享页、点“加入房间”，登录回来以后不用再点就进了房间。
async function joinThroughShareLink(
  visitor: Page,
  code: string,
  catalogId: string,
  credentials: { readonly password: string; readonly username: string },
): Promise<void> {
  await visitor.goto(sharePage(code));
  await visitor.getByRole('link', { name: 'Join room', exact: true }).click();
  await submitIdentityForm(visitor, credentials.username, credentials.password);
  // 回来以后还要登录聊天服务器：再跳去它那边一趟，登录过的人会先看到一页“Continue”，点了才回来。
  const consent = visitor.getByRole('link', { name: /^Continue$/u });
  await expect
    .poll(
      async () => {
        if (await consent.isVisible()) {
          await consent.click().catch(() => undefined);
          return 'consent';
        }
        const here = new URL(visitor.url()).pathname;
        return inRoom(catalogId).test(here) ? 'in the room' : `pending at ${here}`;
      },
      { timeout: 120_000 },
    )
    .toBe('in the room');
}

async function requireApproval(owner: Page): Promise<void> {
  const settings = await openRoomSettings(owner, 'Public');
  await settings
    .getByRole('group', { name: 'How people join' })
    .getByRole('radio', { name: /^Ask to join/u })
    .check();
  await settings.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(settings.getByText('Saved.', { exact: true })).toBeVisible({ timeout: 30_000 });
  await closeRoomSettings(owner, settings);
}

async function leaveRoom(person: Page): Promise<void> {
  const settings = await openRoomSettings(person, 'Members');
  await settings.getByRole('button', { name: 'Leave room', exact: true }).click();
  await expect(person).not.toHaveURL(roomPathPattern, { timeout: 60_000 });
}

async function askToJoin(visitor: Page, code: string, name: string): Promise<void> {
  await visitor.goto(`/explore/${code}`);
  const panel = visitor.getByRole('dialog', { name });
  await panel
    .getByRole('textbox', { name: 'A note for the owner (optional)' })
    .fill('Can I come back in?');
  await panel.getByRole('button', { name: 'Ask to join', exact: true }).click();
  await expect(panel).toContainText('You asked to join.', { timeout: 30_000 });
}

async function approve(owner: Page, displayName: string): Promise<void> {
  const settings = await openRoomSettings(owner, 'Members');
  await expect(settings).toContainText('Can I come back in?', { timeout: 30_000 });
  await settings.getByRole('button', { name: `Approve ${displayName}`, exact: true }).click();
  await expect(settings).not.toContainText('Can I come back in?', { timeout: 30_000 });
  await closeRoomSettings(owner, settings);
}

async function changeShareCode(owner: Page): Promise<string> {
  const settings = await openRoomSettings(owner, 'Public');
  await settings.getByRole('button', { name: 'New share code', exact: true }).click();
  await settings.getByRole('button', { name: 'Change it', exact: true }).click();
  await expect(settings).toContainText('Share code changed.', { timeout: 30_000 });
  const code = shareCodeIn(await settings.getByLabel('Share link', { exact: true }).innerText());
  await closeRoomSettings(owner, settings);
  return code;
}

async function makePrivate(owner: Page): Promise<void> {
  const settings = await openRoomSettings(owner, 'Public');
  await settings
    .getByRole('group', { name: 'Who can find this room' })
    .getByRole('radio', { name: /^Private/u })
    .check();
  await settings.getByRole('button', { name: 'Save', exact: true }).click();
  await settings.getByRole('button', { name: 'Make private', exact: true }).click();
  await expect(settings.getByText('Saved.', { exact: true })).toBeVisible({ timeout: 60_000 });
  await closeRoomSettings(owner, settings);
}

test('公开房间：从发现页和分享链接进来，申请和批准，换分享码，改回私密以后的话是加密的', async ({
  browser,
  playwright,
}) => {
  test.skip(
    ownerUsername === undefined ||
      ownerPassword === undefined ||
      memberUsername === undefined ||
      memberPassword === undefined ||
      visitorUsername === undefined ||
      visitorPassword === undefined ||
      roomName === undefined ||
      saidInPublic === undefined ||
      saidInPrivate === undefined ||
      resultPath === undefined,
    '缺少公开房间纵向验收变量。',
  );
  test.setTimeout(900_000);
  expect(isAbsolute(resultPath ?? '')).toBe(true);
  const name = roomName ?? '';
  const said = { encrypted: saidInPrivate ?? '', plain: saidInPublic ?? '' };

  const contexts: BrowserContext[] = [];
  const api = await playwright.request.newContext({ ignoreHTTPSErrors: true });
  try {
    const owner = await openIsolatedPage(browser, contexts);
    const member = await openIsolatedPage(browser, contexts);
    const visitor = await openIsolatedPage(browser, contexts);
    const failures = {
      member: collectUnhandledFailures(member),
      owner: collectUnhandledFailures(owner),
      visitor: collectUnhandledFailures(visitor),
    };

    await connectLiveSession(owner, {
      expectedDisplayName: 'Local Developer',
      password: ownerPassword ?? '',
      username: ownerUsername ?? '',
    });
    await waitUntilDeviceReady(owner);
    const catalogId = await createPublicRoom(owner, name);

    await connectLiveSession(member, {
      expectedDisplayName: 'Local Collaborator',
      password: memberPassword ?? '',
      username: memberUsername ?? '',
    });
    await waitUntilDeviceReady(member);
    await joinFromExplore(member, name, catalogId);
    await openConversation(member);
    await say(member, said.plain);
    await openConversation(owner);
    await expectSaid(owner, said.plain);

    const firstCode = await readShareCode(member);
    await expectSharePage(api, firstCode, name);
    await joinThroughShareLink(visitor, firstCode, catalogId, {
      password: visitorPassword ?? '',
      username: visitorUsername ?? '',
    });

    // 改成要申请。丙先退出来，这一次要申请、等批准；批准以后他的网页自己进来，不用再点。
    await requireApproval(owner);
    await leaveRoom(visitor);
    // 退出时还在路上的请求可能被拒，这时候之前的看过就清掉，这一段不算。
    expect(failures.visitor).toEqual([]);
    failures.visitor.length = 0;
    await askToJoin(visitor, firstCode, name);
    await approve(owner, 'Local Visitor');
    await expect(visitor).toHaveURL(inRoom(catalogId), { timeout: 90_000 });

    const secondCode = await changeShareCode(owner);
    await expectShareGone(api, firstCode, name);
    await expectSharePage(api, secondCode, name);

    await makePrivate(owner);
    await expectShareGone(api, secondCode, name);
    await say(owner, said.encrypted);
    await expectSaid(member, said.encrypted);
    expect(failures).toEqual({ member: [], owner: [], visitor: [] });
    await writeFile(resultPath ?? '', `${JSON.stringify({ catalogId })}\n`, 'utf8');
  } finally {
    await api.dispose();
    await Promise.all(
      contexts.map(async (context) => {
        await context.close();
      }),
    );
  }
});
