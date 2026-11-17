import { readFile, rename, writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import {
  expect,
  test,
  type APIRequestContext,
  type BrowserContext,
  type Locator,
  type Page,
} from '@playwright/test';
import { z } from 'zod';

import {
  apiOrigin,
  collectUnhandledFailures,
  connectLiveSession,
} from '../e2e-live/support/live-session';
import {
  closeRoomSettings,
  conversationLog,
  expectSaid,
  inRoom,
  openConversation,
  openIsolatedPage,
  openRoomSettings,
  waitForStep,
  waitUntilDeviceReady,
} from './support/room-pages';

// 分享页放实况和分享一段对话（specs/redesign/project-rooms.md 的 3P-9b-1、3P-9b-2）。项目房间是纵向验收里
// 验收账号用命令行认领、认上以后建的（tools/vertical_project_rooms.py），建好就开着实况。房主在真实网页的
// 房间设置里关掉、再打开，纵向验收在关着时、再打开以后各让他的本机 Agent 说一句，不登录看 `/r/<码>/live` 和
// `/gh/` 页。Agent 接着又说两句，房主在对话里从再打开以后那句选到最后一句分享出去，经接口试一段从关着时那句
// 开始的（被拒），再分享最后一句留着；纵向验收不登录打开这两段，房主在房间的“分享”里删掉第一段。然后房主经
// 控制面的接口把房间改成要申请：实况自己关了、开也开不了，分享过的一段也没了；改回直接进，实况也不自己回来。
// 最后是公开房间那一轮他建的、最后改回私密的房间：私密时开不了，再设成公开（加密的）也开不了，不登录都看不到
// 实况；看完改回私密。
const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const projectRoomId = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_ROOM;
const formerPublicRoomId = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_FORMER_PUBLIC_ROOM;
// 和纵向验收靠这几个文件对步子。
const offPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_OFF;
const saidOffPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_SAID_OFF;
const onPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_ON;
const seenOnPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_SEEN_ON;
const sharedPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_SHARED;
const seenSharedPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_SEEN_SHARED;
const deletedPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_DELETED;
const seenDeletedPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_SEEN_DELETED;
const requestPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_REQUEST;
const stepPaths = [
  offPath,
  saidOffPath,
  onPath,
  seenOnPath,
  sharedPath,
  seenSharedPath,
  deletedPath,
  seenDeletedPath,
  requestPath,
];

const switchName = 'Show what’s said here on the share page';
const liveLine = 'Visible without signing in';
const shareAction = 'Share this stretch';
// 分享出去的一段对话：挂在这间房的分享链接下面。
const excerptLink = /^https:\/\/[^/\s]+\/r\/[A-Za-z0-9]{6}\/s\/[A-Za-z0-9]{8}$/u;

type Answer = { readonly status: number; readonly body: unknown };

const liveViewSchema = z
  .object({
    enabled: z.boolean(),
    enabledAtUnixMs: z.number().nullable(),
    live: z.boolean(),
    unmetConditions: z.array(z.string()),
    canChange: z.boolean(),
  })
  .strict();
type LiveView = z.infer<typeof liveViewSchema>;

const listingSchema = z
  .object({
    listed: z.boolean(),
    joinMode: z.enum(['open', 'request', 'invite_only']),
    memberPermissions: z.object({ capabilities: z.array(z.string()) }).strict(),
    allowNetworkAgents: z.boolean(),
    shareCode: z.string().nullable(),
    endToEndEncrypted: z.boolean(),
  })
  .loose();
type Listing = z.infer<typeof listingSchema>;

const errorSchema = z
  .object({ code: z.string(), details: z.record(z.string(), z.unknown()) })
  .loose();

// Agent 说的一句（纵向验收写在“看到再打开以后那句了”的文件里）。
const saidSchema = z
  .object({ eventId: z.string().startsWith('$'), text: z.string().min(1) })
  .strict();
type Said = z.infer<typeof saidSchema>;
// 关着实况时说的那句，和要分享的那一段的头尾、几句。
const stretchSchema = z
  .object({ before: saidSchema, first: saidSchema, last: saidSchema, count: z.number().int() })
  .strict();
type Stretch = z.infer<typeof stretchSchema>;

const sharedExcerptSchema = z
  .object({ code: z.string().regex(/^[A-Za-z0-9]{8}$/u), url: z.string().regex(excerptLink) })
  .strict();
const excerptListSchema = z
  .object({
    excerpts: z.array(
      z
        .object({
          code: z.string(),
          url: z.string().nullable(),
          messageCount: z.number(),
          firstMessageAtUnixMs: z.number(),
          sharedAtUnixMs: z.number(),
          mine: z.boolean(),
        })
        .strict(),
    ),
  })
  .strict();
type ListedExcerpt = z.infer<typeof excerptListSchema>['excerpts'][number];

// 先写旁边的临时文件再换名：纵向验收看到文件时内容已经写全了。
async function markStep(path: string | undefined, payload: unknown = {}): Promise<void> {
  const target = path ?? '';
  await writeFile(`${target}.tmp`, `${JSON.stringify(payload)}\n`, 'utf8');
  await rename(`${target}.tmp`, target);
}

/** 页面里带着登录发一个控制面请求，回答状态和正文（没有正文时为 null）。 */
async function request(page: Page, path: string, method: string, body?: unknown): Promise<Answer> {
  return await page.evaluate(
    async ({ base, target, verb, payload }) => {
      const response = await fetch(`${base}${target}`, {
        cache: 'no-store',
        credentials: 'include',
        headers: {
          Accept: 'application/json',
          ...(payload === null ? {} : { 'Content-Type': 'application/json' }),
        },
        method: verb,
        ...(payload === null ? {} : { body: JSON.stringify(payload) }),
      });
      const text = await response.text();
      return { body: text === '' ? null : (JSON.parse(text) as unknown), status: response.status };
    },
    { base: apiOrigin, target: path, verb: method, payload: body ?? null },
  );
}

async function readLiveView(page: Page, catalogId: string): Promise<LiveView> {
  const answer = await request(page, `/private-rooms/${catalogId}/live-view`, 'GET');
  expect(answer.status).toBe(200);
  return liveViewSchema.parse(answer.body);
}

// 开不了：409 `room_live.not_eligible`，`details.condition` 是排在最前的那一样。
async function expectRefused(page: Page, catalogId: string, condition: string): Promise<void> {
  const answer = await request(page, `/private-rooms/${catalogId}/live-view`, 'PUT', {
    enabled: true,
  });
  expect(answer.status).toBe(409);
  const refusal = errorSchema.parse(answer.body);
  expect(refusal.code).toBe('room_live.not_eligible');
  expect(refusal.details.condition).toBe(condition);
}

async function readListing(page: Page, catalogId: string): Promise<Listing> {
  const answer = await request(page, `/private-rooms/${catalogId}/listing`, 'GET');
  expect(answer.status).toBe(200);
  return listingSchema.parse(answer.body);
}

// 公开设置一次存整份：别的照旧，只改这一两样。
async function saveListing(
  page: Page,
  catalogId: string,
  current: Listing,
  change: { readonly listed?: boolean; readonly joinMode?: Listing['joinMode'] },
): Promise<Listing> {
  const answer = await request(page, `/private-rooms/${catalogId}/listing`, 'PUT', {
    listed: change.listed ?? current.listed,
    joinMode: change.joinMode ?? current.joinMode,
    memberPermissions: current.memberPermissions,
    allowNetworkAgents: current.allowNetworkAgents,
  });
  expect(answer.status).toBe(200);
  return listingSchema.parse(answer.body);
}

// 不登录看：不放实况的码回 404 `room_live.unavailable`，不说是哪一种。
async function expectNotLive(api: APIRequestContext, code: string): Promise<void> {
  const answer = await api.get(`${apiOrigin}/r/${code}/live`);
  expect(answer.status()).toBe(404);
  expect(errorSchema.parse(await answer.json()).code).toBe('room_live.unavailable');
}

/** 顶栏房间名旁边那一小句（窄的时候只剩后一半）。 */
function topBarLine(page: Page): Locator {
  return page.locator('.workspace-header__notice').filter({ visible: true });
}

async function liveSwitch(page: Page): Promise<{ settings: Locator; group: Locator }> {
  const settings = await openRoomSettings(page, 'Public');
  const group = settings.getByRole('region', { name: 'Live view on the share page' });
  await group.scrollIntoViewIfNeeded();
  return { settings, group };
}

// 关掉不用确认。
async function turnOff(page: Page): Promise<void> {
  const { settings, group } = await liveSwitch(page);
  const toggle = group.getByRole('switch', { name: switchName });
  await expect(toggle).toBeChecked({ timeout: 60_000 });
  await expect(toggle).toHaveAccessibleDescription(
    /^On since .+\. Only what’s said after that is shown\.$/u,
  );
  await toggle.click();
  await expect(toggle).not.toBeChecked({ timeout: 60_000 });
  await expect(toggle).toHaveAccessibleDescription('Off.');
  await closeRoomSettings(page, settings);
  await expect(page.getByText(liveLine)).toHaveCount(0, { timeout: 60_000 });
}

// 打开先在下面确认。
async function turnOn(page: Page): Promise<void> {
  const { settings, group } = await liveSwitch(page);
  const toggle = group.getByRole('switch', { name: switchName });
  await expect(toggle).not.toBeChecked({ timeout: 60_000 });
  await toggle.click();
  await group.getByRole('button', { name: 'Turn on', exact: true }).click();
  await expect(toggle).toBeChecked({ timeout: 60_000 });
  await expect(toggle).toHaveAccessibleDescription(
    /^On since .+\. Only what’s said after that is shown\.$/u,
  );
  await closeRoomSettings(page, settings);
  await expect(topBarLine(page)).toContainText(liveLine, { timeout: 60_000 });
}

function excerptsPath(catalogId: string): string {
  return `/private-rooms/${catalogId}/excerpts`;
}

function messagesLabel(count: number): string {
  return count === 1 ? '1 message' : `${String(count)} messages`;
}

/** 对话里说这句话的那一条（连着外面那一层，悬停出来的那一排在里面）。 */
function messageSaying(page: Page, text: string): Locator {
  return conversationLog(page).locator('[data-conversation-message-id]').filter({ hasText: text });
}

async function hoverOver(message: Locator): Promise<void> {
  await message.scrollIntoViewIfNeeded();
  await message.hover();
}

// 房间页读到实况开着以后，开了实况以后说的话上才有“分享这一段”；关着时说的那句一直没有。
async function onlyLiveMessagesShare(page: Page, said: Stretch): Promise<void> {
  const start = messageSaying(page, said.first.text);
  await hoverOver(start);
  await expect(start.getByRole('button', { name: shareAction, exact: true })).toBeVisible({
    timeout: 60_000,
  });
  const before = messageSaying(page, said.before.text);
  await hoverOver(before);
  await expect(before.getByRole('button', { name: /^Reply to /u })).toBeVisible();
  await expect(before.getByRole('button', { name: shareAction, exact: true })).toHaveCount(0);
}

// 在对话里选一段分享：再打开实况以后那句是开头，点 Agent 说的最后一句是结尾，“分享”，从对话框里读出链接。
async function shareInConversation(page: Page, said: Stretch): Promise<string> {
  // 这台设备不把 Agent 之间连着的话折起来（`agent-run-folding.ts`），那几句一句一句摆着。
  await page.evaluate(() => {
    localStorage.setItem('agent-room.conversation.agent-run-folding', 'off');
  });
  await openConversation(page);
  await expectSaid(page, said.last.text);
  await onlyLiveMessagesShare(page, said);

  const start = messageSaying(page, said.first.text);
  await hoverOver(start);
  await start.getByRole('button', { name: shareAction, exact: true }).click();
  const bar = page.getByRole('region', { name: 'Share a stretch of conversation' });
  await expect(bar).toContainText('1 selected');
  const end = messageSaying(page, said.last.text);
  await end.scrollIntoViewIfNeeded();
  await end.getByRole('button', { name: /^Select this message:/u }).click();
  await expect(bar).toContainText(`${String(said.count)} selected`);
  await bar.getByRole('button', { name: 'Share', exact: true }).click();

  const dialog = page.getByRole('dialog', { name: 'Share this stretch' });
  const link = dialog.getByRole('textbox', { name: 'Link' });
  await expect(link).toHaveValue(excerptLink, { timeout: 60_000 });
  const preview = dialog.getByRole('list', {
    name: new RegExp(`^${messagesLabel(said.count)} · `, 'u'),
  });
  await expect(preview.getByRole('listitem')).toHaveCount(said.count);
  await expect(preview.getByRole('listitem').first()).toContainText(said.first.text);
  await expect(preview.getByRole('listitem').last()).toContainText(said.last.text);
  const url = await link.inputValue();
  // 关掉对话框这一段就分享完了，不再选。
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(bar).toHaveCount(0);
  return url;
}

// 开头早于这次开实况（关着时说的那句）：422 `room_excerpt.invalid_range`，`details.reason` 是 `before_live`。
async function expectBeforeLiveRefused(
  page: Page,
  catalogId: string,
  said: Stretch,
): Promise<void> {
  const answer = await request(page, excerptsPath(catalogId), 'POST', {
    firstEventId: said.before.eventId,
    lastEventId: said.first.eventId,
  });
  expect(answer.status).toBe(422);
  const refusal = errorSchema.parse(answer.body);
  expect(refusal.code).toBe('room_excerpt.invalid_range');
  expect(refusal.details.reason).toBe('before_live');
}

async function shareOne(page: Page, catalogId: string, said: Said): Promise<string> {
  const answer = await request(page, excerptsPath(catalogId), 'POST', {
    firstEventId: said.eventId,
    lastEventId: said.eventId,
  });
  expect(answer.status).toBe(201);
  return sharedExcerptSchema.parse(answer.body).url;
}

async function listExcerpts(page: Page, catalogId: string): Promise<readonly ListedExcerpt[]> {
  const answer = await request(page, excerptsPath(catalogId), 'GET');
  expect(answer.status).toBe(200);
  return excerptListSchema.parse(answer.body).excerpts;
}

// 一段对话：对话里选的那一段；开头太早的被拒；最后一句单独经接口分享、留到实况关掉。房主看得到两段，新的在前。
async function shareStretches(
  page: Page,
  catalogId: string,
  said: Stretch,
): Promise<{ shown: string; kept: string }> {
  const shown = await shareInConversation(page, said);
  await expectBeforeLiveRefused(page, catalogId, said);
  const kept = await shareOne(page, catalogId, said.last);
  const listed = await listExcerpts(page, catalogId);
  expect(listed.map(({ url, messageCount, mine }) => ({ url, messageCount, mine }))).toEqual([
    { url: kept, messageCount: 1, mine: true },
    { url: shown, messageCount: said.count, mine: true },
  ]);
  return { shown, kept };
}

// 在房间的“分享”里删掉对话里选的那一段：先在这一行里确认，删了只剩另一段。
async function deleteInRoomShare(
  page: Page,
  catalogId: string,
  excerpts: { readonly shown: string; readonly kept: string; readonly count: number },
): Promise<void> {
  await page
    .getByRole('button', { name: 'Share', exact: true })
    .filter({ visible: true })
    .first()
    .click();
  const dialog = page.getByRole('dialog', { name: 'Share this room' });
  const shared = dialog.getByRole('region', { name: 'Shared from this room' });
  await shared.scrollIntoViewIfNeeded();
  const rows = shared.getByRole('group');
  await expect(rows).toHaveCount(2, { timeout: 60_000 });
  const row = rows.filter({ has: page.locator(`a[href="${excerpts.shown}"]`) });
  await expect(row).toContainText(messagesLabel(excerpts.count));
  await expect(row).toContainText('Yours');
  await row.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(row).toContainText('Delete it? The link stops working right away.');
  await row.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(shared.getByRole('status')).toHaveText('Deleted. The link no longer opens.');
  await expect(rows).toHaveCount(1);
  await expect(rows.first().locator(`a[href="${excerpts.kept}"]`)).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  expect((await listExcerpts(page, catalogId)).map(({ url }) => url)).toEqual([excerpts.kept]);
}

// 改成要申请：实况自己关了（那一行删了），分享过的段一起删了，开也开不了；改回直接进，实况不自己回来。
async function joinModeTurnsLiveOff(page: Page, catalogId: string): Promise<void> {
  const before = await readListing(page, catalogId);
  expect(before).toMatchObject({ listed: true, joinMode: 'open', endToEndEncrypted: false });
  expect(await readLiveView(page, catalogId)).toMatchObject({ enabled: true, live: true });
  const asking = await saveListing(page, catalogId, before, { joinMode: 'request' });
  expect(await readLiveView(page, catalogId)).toEqual({
    enabled: false,
    enabledAtUnixMs: null,
    live: false,
    unmetConditions: ['join_mode'],
    canChange: true,
  });
  expect(await listExcerpts(page, catalogId)).toEqual([]);
  await expectRefused(page, catalogId, 'join_mode');
  await markStep(requestPath);
  await saveListing(page, catalogId, asking, { joinMode: 'open' });
  expect(await readLiveView(page, catalogId)).toEqual({
    enabled: false,
    enabledAtUnixMs: null,
    live: false,
    unmetConditions: [],
    canChange: true,
  });
}

// 私密的房间开不了，再设成公开（加密只能开、不能关）也开不了；不登录都看不到实况。看完改回私密。
async function privateRoomsStayUnreadable(
  page: Page,
  api: APIRequestContext,
  catalogId: string,
): Promise<void> {
  const unlisted = await readListing(page, catalogId);
  expect(unlisted).toMatchObject({ listed: false, endToEndEncrypted: true });
  // 公开过的房间记着分享码。
  const code = unlisted.shareCode ?? '';
  expect(code).toMatch(/^[A-Za-z0-9]{6}$/u);
  const privately = await readLiveView(page, catalogId);
  expect(privately).toMatchObject({ enabled: false, live: false, canChange: true });
  expect(privately.unmetConditions[0]).toBe('not_listed');
  expect(privately.unmetConditions).toContain('encrypted');
  await expectRefused(page, catalogId, 'not_listed');
  await expectNotLive(api, code);

  const listed = await saveListing(page, catalogId, unlisted, { listed: true, joinMode: 'open' });
  expect(listed).toMatchObject({ listed: true, joinMode: 'open', endToEndEncrypted: true });
  expect(listed.shareCode).toBe(code);
  expect(await readLiveView(page, catalogId)).toEqual({
    enabled: false,
    enabledAtUnixMs: null,
    live: false,
    unmetConditions: ['encrypted'],
    canChange: true,
  });
  await expectRefused(page, catalogId, 'encrypted');
  const sharePage = await api.get(`${apiOrigin}/r/${code}`, {
    headers: { 'Accept-Language': 'en' },
  });
  expect(sharePage.status()).toBe(200);
  expect(await sharePage.text()).not.toContain('data-live=');
  await expectNotLive(api, code);
  await saveListing(page, catalogId, listed, { listed: false });
}

// 没有这个上限时，一个动作找不到东西会一直等，最后只看到外面的脚本超时，不知道卡在哪。
test.use({ actionTimeout: 90_000 });

test('分享页放实况：房主关掉再打开、分享一段对话又删掉、改成要申请；私密的和加密的公开房间开不了', async ({
  browser,
  playwright,
}) => {
  test.skip(
    username === undefined ||
      password === undefined ||
      projectRoomId === undefined ||
      formerPublicRoomId === undefined ||
      stepPaths.some((path) => path === undefined),
    '缺少分享页放实况纵向验收变量。',
  );
  test.setTimeout(1_500_000);
  for (const path of stepPaths) expect(isAbsolute(path ?? '')).toBe(true);
  const project = projectRoomId ?? '';
  const former = formerPublicRoomId ?? '';
  // 故意去开、开不了的那几次 409，和故意分享一段开头太早的那次 422。
  const refusals = new Set([
    ...[project, former].map((id) => `HTTP 409 ${apiOrigin}/private-rooms/${id}/live-view`),
    `HTTP 422 ${apiOrigin}${excerptsPath(project)}`,
  ]);

  const contexts: BrowserContext[] = [];
  const api = await playwright.request.newContext({ ignoreHTTPSErrors: true });
  try {
    const owner = await openIsolatedPage(browser, contexts);
    const failures = collectUnhandledFailures(owner);
    await connectLiveSession(owner, {
      expectedDisplayName: 'Local Developer',
      password: password ?? '',
      username: username ?? '',
    });
    await waitUntilDeviceReady(owner);
    await owner.goto(`/lobby/${project}`);
    await expect(owner).toHaveURL(inRoom(project), { timeout: 90_000 });
    await expect(topBarLine(owner)).toContainText(liveLine, { timeout: 60_000 });

    await turnOff(owner);
    await markStep(offPath);
    await waitForStep(saidOffPath ?? '');
    await turnOn(owner);
    await markStep(onPath);

    // 纵向验收看到再打开以后的那句了，Agent 又接着说了两句：在对话里分享一段，经接口再分享一段。
    await waitForStep(seenOnPath ?? '');
    const said = stretchSchema.parse(JSON.parse(await readFile(seenOnPath ?? '', 'utf8')));
    const excerpts = await shareStretches(owner, project, said);
    await markStep(sharedPath, excerpts);

    // 纵向验收不登录打开了这两段：在房间的“分享”里删掉对话里选的那一段。
    await waitForStep(seenSharedPath ?? '');
    await deleteInRoomShare(owner, project, { ...excerpts, count: said.count });
    await markStep(deletedPath);

    // 纵向验收看到删掉的那一段打不开了、另一段还在。接下来走控制面的接口，先离开房间页。
    await waitForStep(seenDeletedPath ?? '');
    await owner.goto('/rooms');
    await joinModeTurnsLiveOff(owner, project);
    await privateRoomsStayUnreadable(owner, api, former);
    expect(failures.filter((failure) => !refusals.has(failure))).toEqual([]);
  } finally {
    await api.dispose();
    await Promise.all(
      contexts.map(async (context) => {
        await context.close();
      }),
    );
  }
});
