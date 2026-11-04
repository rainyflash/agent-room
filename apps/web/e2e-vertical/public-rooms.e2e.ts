import { access, readFile, writeFile } from 'node:fs/promises';
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
import { z } from 'zod';

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
const firstNotice = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_NOTICE_FIRST;
const secondNotice = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_NOTICE_SECOND;
const agentName = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_AGENT;
const moderationRules = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_RULES;
const spamLine = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_SPAM;
// 和纵向验收靠这几个文件对步子。
const noticedPath = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_NOTICED;
const agentReadFirstPath = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_AGENT_READ_FIRST;
const agentReadSecondPath = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_AGENT_READ_SECOND;
const grantedPath = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_GRANTED;
const mutedPath = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_MUTED;
const revokedPath = process.env.AGENT_ROOM_VERTICAL_PUBLIC_ROOM_REVOKED;

const roomPathPattern = /\/lobby\/([0-9a-f-]{36})\/instance\//u;
const shareCodePattern = /\/r\/([A-Za-z0-9]{6})/u;
const mutedSchema = z.object({ note: z.string().min(1) }).strict();

async function waitForStep(path: string): Promise<void> {
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

async function markStep(path: string): Promise<void> {
  await writeFile(path, `${JSON.stringify({ done: true })}\n`, 'utf8');
}

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

async function writeNotice(owner: Page, text: string): Promise<void> {
  const settings = await openRoomSettings(owner, 'Notice');
  await settings.getByRole('textbox', { name: 'Notice', exact: true }).fill(text);
  await settings.getByRole('button', { name: 'Save notice', exact: true }).click();
  await expect(settings.getByText('Notice updated.', { exact: true })).toBeVisible({
    timeout: 30_000,
  });
  await closeRoomSettings(owner, settings);
}

// 甲写公告，乙的网页上马上看到；本机 Agent 进房间时拿到；甲改了以后，它下一批消息带着新的那条。
async function noticeRound(owner: Page, member: Page, catalogId: string): Promise<void> {
  await writeNotice(owner, firstNotice ?? '');
  await expect(member.getByRole('region', { name: 'Room notice', exact: true })).toContainText(
    firstNotice ?? '',
    { timeout: 30_000 },
  );
  await writeFile(noticedPath ?? '', `${JSON.stringify({ catalogId })}\n`, 'utf8');
  await waitForStep(agentReadFirstPath ?? '');
  await writeNotice(owner, secondNotice ?? '');
  await expect(member.getByRole('region', { name: 'Room notice', exact: true })).toContainText(
    secondNotice ?? '',
    { timeout: 30_000 },
  );
  // 公告改了不叫醒 Agent，跟着下一批消息交。
  await say(member, 'The notice changed, did everyone see it?');
  await waitForStep(agentReadSecondPath ?? '');
}

async function handOverToAgent(moderator: Page, name: string, rules: string): Promise<void> {
  const settings = await openRoomSettings(moderator, 'Moderation');
  const form = settings.locator('.moderation-agent-grant');
  const agent = form.getByLabel('Agent', { exact: true });
  const value = await agent.locator('option', { hasText: name }).getAttribute('value');
  await agent.selectOption(value ?? '');
  await form.getByLabel('Rules', { exact: true }).fill(rules);
  await form.getByRole('button', { name: 'Hand it over', exact: true }).click();
  await expect(
    settings.getByRole('region', { name: 'Agents moderating this room', exact: true }),
  ).toContainText(name, { timeout: 30_000 });
  await closeRoomSettings(moderator, settings);
}

// 台账写着是哪个 Agent 照规则做的；甲一次全撤，再把授权收回来。
async function undoAndRevoke(moderator: Page, name: string, note: string): Promise<void> {
  const settings = await openRoomSettings(moderator, 'Moderation');
  await expect(settings).toContainText(
    new RegExp(`${name} #\\d{4}, following the rules: ${note}`, 'u'),
    { timeout: 30_000 },
  );
  const inEffect = settings.getByRole('region', {
    name: 'Still in effect from agents',
    exact: true,
  });
  await expect(inEffect).toContainText('1 action still in effect', { timeout: 30_000 });
  await inEffect.getByRole('button', { name: 'Undo all', exact: true }).click();
  await inEffect.getByRole('group').getByRole('button', { name: 'Undo all', exact: true }).click();
  await expect(inEffect).toContainText('Undid 1 action.', { timeout: 60_000 });
  const grants = settings.getByRole('region', {
    name: 'Agents moderating this room',
    exact: true,
  });
  await grants.getByRole('button', { name: 'Revoke', exact: true }).click();
  await expect(grants).toContainText('No agent is moderating this room.', { timeout: 30_000 });
  await closeRoomSettings(moderator, settings);
}

// 甲把房间交给自己的 Agent，规则是说 spam 的禁言；乙说了，Agent 照规则禁言他，甲再撤回。
async function agentModerationRound(moderator: Page, member: Page, room: string): Promise<void> {
  await moderator.goto(room);
  await handOverToAgent(moderator, agentName ?? '', moderationRules ?? '');
  await markStep(grantedPath ?? '');
  await say(member, spamLine ?? '');

  await waitForStep(mutedPath ?? '');
  const { note } = mutedSchema.parse(JSON.parse(await readFile(mutedPath ?? '', 'utf8')));
  const box = member
    .getByRole('region', { name: 'Conversation', exact: true })
    .getByRole('textbox', { name: 'Message', exact: true });
  await expect(box).toBeDisabled({ timeout: 60_000 });

  await moderator.reload();
  await undoAndRevoke(moderator, agentName ?? '', note);
  await expect(box).toBeEnabled({ timeout: 60_000 });
  await markStep(revokedPath ?? '');
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
      resultPath === undefined ||
      firstNotice === undefined ||
      secondNotice === undefined ||
      agentName === undefined ||
      moderationRules === undefined ||
      spamLine === undefined ||
      noticedPath === undefined ||
      agentReadFirstPath === undefined ||
      agentReadSecondPath === undefined ||
      grantedPath === undefined ||
      mutedPath === undefined ||
      revokedPath === undefined,
    '缺少公开房间纵向验收变量。',
  );
  test.setTimeout(1_500_000);
  for (const path of [
    resultPath,
    noticedPath,
    agentReadFirstPath,
    agentReadSecondPath,
    grantedPath,
    mutedPath,
    revokedPath,
  ]) {
    expect(isAbsolute(path ?? '')).toBe(true);
  }
  const name = roomName ?? '';
  const said = { encrypted: saidInPrivate ?? '', plain: saidInPublic ?? '' };

  const contexts: BrowserContext[] = [];
  const api = await playwright.request.newContext({ ignoreHTTPSErrors: true });
  try {
    const owner = await openIsolatedPage(browser, contexts);
    const member = await openIsolatedPage(browser, contexts);
    const visitor = await openIsolatedPage(browser, contexts);
    const moderator = await openIsolatedPage(browser, contexts);
    const failures = {
      member: collectUnhandledFailures(member),
      moderator: collectUnhandledFailures(moderator),
      owner: collectUnhandledFailures(owner),
      visitor: collectUnhandledFailures(visitor),
    };

    const ownerCredentials = {
      expectedDisplayName: 'Local Developer',
      password: ownerPassword ?? '',
      username: ownerUsername ?? '',
    };
    await connectLiveSession(owner, ownerCredentials);
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
    // 退出那一刻房间页还在读这间房的设置和成员，人已经不在了，回 404 是正常的。
    // 所以退出之前先查一遍，重新打开页面、路上的请求都落定了再清。
    expect(failures.visitor).toEqual([]);
    await leaveRoom(visitor);
    await waitUntilDeviceReady(visitor);
    failures.visitor.length = 0;
    await askToJoin(visitor, firstCode, name);
    await approve(owner, 'Local Visitor');
    await expect(visitor).toHaveURL(inRoom(catalogId), { timeout: 90_000 });

    await noticeRound(owner, member, catalogId);
    // 给授权、一次全撤都要最近登录过（5 分钟）。走到这里已经花了好几分钟，换一个刚登录的页面来做。
    await connectLiveSession(moderator, ownerCredentials);
    await waitUntilDeviceReady(moderator);
    await agentModerationRound(moderator, member, new URL(owner.url()).pathname);

    const secondCode = await changeShareCode(owner);
    await expectShareGone(api, firstCode, name);
    await expectSharePage(api, secondCode, name);

    await makePrivate(owner);
    await expectShareGone(api, secondCode, name);
    await say(owner, said.encrypted);
    await expectSaid(member, said.encrypted);
    expect(failures).toEqual({ member: [], moderator: [], owner: [], visitor: [] });
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
