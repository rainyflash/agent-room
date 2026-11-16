import { writeFile } from 'node:fs/promises';
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
  inRoom,
  openIsolatedPage,
  openRoomSettings,
  waitForStep,
  waitUntilDeviceReady,
} from './support/room-pages';

// 分享页放实况（specs/redesign/project-rooms.md 的 3P-9b-1）。项目房间是纵向验收里验收账号用命令行认领、
// 认上以后建的（tools/vertical_project_rooms.py），建好就开着实况。房主在真实网页的房间设置里关掉、再打开，
// 纵向验收在关着时、再打开以后各让他的本机 Agent 说一句，不登录看 `/r/<码>/live` 和 `/gh/` 页。然后房主经
// 控制面的接口把房间改成要申请：实况自己关了、开也开不了；改回直接进，实况也不自己回来。最后是公开房间那一轮
// 他建的、最后改回私密的房间：私密时开不了，再设成公开（加密的）也开不了，不登录都看不到实况；看完改回私密。
const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const projectRoomId = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_ROOM;
const formerPublicRoomId = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_FORMER_PUBLIC_ROOM;
// 和纵向验收靠这几个文件对步子。
const offPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_OFF;
const saidOffPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_SAID_OFF;
const onPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_ON;
const seenOnPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_SEEN_ON;
const requestPath = process.env.AGENT_ROOM_VERTICAL_PROJECT_LIVE_REQUEST;
const stepPaths = [offPath, saidOffPath, onPath, seenOnPath, requestPath];

const switchName = 'Show what’s said here on the share page';
const liveLine = 'Visible without signing in';

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

async function markStep(path: string | undefined): Promise<void> {
  await writeFile(path ?? '', '{}\n', 'utf8');
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

// 改成要申请：实况自己关了（那一行删了），开也开不了；改回直接进，实况不自己回来。
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

test('分享页放实况：房主关掉再打开、改成要申请；私密的和加密的公开房间开不了', async ({
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
  test.setTimeout(900_000);
  for (const path of stepPaths) expect(isAbsolute(path ?? '')).toBe(true);
  const project = projectRoomId ?? '';
  const former = formerPublicRoomId ?? '';
  // 故意去开、开不了的那几次 409。
  const refusals = new Set(
    [project, former].map((id) => `HTTP 409 ${apiOrigin}/private-rooms/${id}/live-view`),
  );

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

    // 纵向验收看到再打开以后的那句了。接下来走控制面的接口，先离开房间页。
    await waitForStep(seenOnPath ?? '');
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
