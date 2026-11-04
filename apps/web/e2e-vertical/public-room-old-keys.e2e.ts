import { writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import { expect, test, type BrowserContext, type Page } from '@playwright/test';

import { collectUnhandledFailures, connectLiveSession } from '../e2e-live/support/live-session';
import {
  closeRoomSettings,
  conversationLog,
  expectSaid,
  joinFromExplore,
  openConversation,
  openIsolatedPage,
  openRoomSettings,
  roomPathPattern,
  say,
  waitForStep,
  waitUntilDeviceReady,
} from './support/room-pages';

const ownerUsername = process.env.AGENT_ROOM_VERTICAL_OLD_KEYS_OWNER_USERNAME;
const ownerPassword = process.env.AGENT_ROOM_VERTICAL_OLD_KEYS_OWNER_PASSWORD;
const newcomerUsername = process.env.AGENT_ROOM_E2E_USERNAME;
const newcomerPassword = process.env.AGENT_ROOM_E2E_PASSWORD;
const roomName = process.env.AGENT_ROOM_VERTICAL_OLD_KEYS_ROOM;
const agentName = process.env.AGENT_ROOM_VERTICAL_OLD_KEYS_AGENT;
const saidBefore = process.env.AGENT_ROOM_VERTICAL_OLD_KEYS_SAID_BEFORE;
const saidAfter = process.env.AGENT_ROOM_VERTICAL_OLD_KEYS_SAID_AFTER;
const joinedPath = process.env.AGENT_ROOM_VERTICAL_OLD_KEYS_JOINED;
const agentInPath = process.env.AGENT_ROOM_VERTICAL_OLD_KEYS_AGENT_IN;
const checkedPath = process.env.AGENT_ROOM_VERTICAL_OLD_KEYS_CHECKED;

// 新建时不选公开：私密的，端到端加密。
async function createPrivateRoom(owner: Page, name: string): Promise<string> {
  await owner.getByRole('link', { name: 'Rooms', exact: true }).click();
  await owner.getByRole('button', { name: 'New room', exact: true }).click();
  const dialog = owner.getByRole('dialog', { name: 'New room' });
  await dialog.getByLabel('Name', { exact: true }).fill(name);
  await dialog.getByRole('button', { name: 'Create and enter', exact: true }).click();
  await expect(owner).toHaveURL(roomPathPattern, { timeout: 90_000 });
  const catalogId = roomPathPattern.exec(owner.url())?.[1];
  if (catalogId === undefined) throw new Error('建好以后不在房间里。');
  return catalogId;
}

async function makePublic(owner: Page): Promise<void> {
  const settings = await openRoomSettings(owner, 'Public');
  await settings
    .getByRole('group', { name: 'Who can find this room' })
    .getByRole('radio', { name: /^Public/u })
    .check();
  await settings.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(settings).toContainText('people who join later won’t see earlier messages');
  await settings.getByRole('button', { name: 'Make public', exact: true }).click();
  await expect(settings.getByText('Saved.', { exact: true })).toBeVisible({ timeout: 60_000 });
  await closeRoomSettings(owner, settings);
}

// 房主的网页同步到 Agent 进来了，接下来这句话的钥匙才发得到它那里。
async function waitForAgentInRoster(owner: Page, name: string): Promise<void> {
  const room = new URL(owner.url());
  room.searchParams.delete('view');
  await owner.goto(`${room.pathname}${room.search}`);
  await expect(
    owner.getByRole('option', { name: new RegExp(`^${name} #\\d{4}, `, 'u') }),
  ).toHaveCount(1, { timeout: 120_000 });
}

// 没有这个上限时，一个动作找不到东西会一直等，最后只看到外面的脚本超时，不知道卡在哪。
test.use({ actionTimeout: 90_000 });

test('私密房间改成公开：后来的人和他的 Agent 读不到改之前说的话，读得到之后说的', async ({
  browser,
}) => {
  test.skip(
    ownerUsername === undefined ||
      ownerPassword === undefined ||
      newcomerUsername === undefined ||
      newcomerPassword === undefined ||
      roomName === undefined ||
      agentName === undefined ||
      saidBefore === undefined ||
      saidAfter === undefined ||
      joinedPath === undefined ||
      agentInPath === undefined ||
      checkedPath === undefined,
    '缺少私密改公开纵向验收变量。',
  );
  test.setTimeout(900_000);
  for (const path of [joinedPath, agentInPath, checkedPath]) {
    expect(isAbsolute(path ?? '')).toBe(true);
  }
  const name = roomName ?? '';

  const contexts: BrowserContext[] = [];
  try {
    const owner = await openIsolatedPage(browser, contexts);
    const newcomer = await openIsolatedPage(browser, contexts);
    const failures = {
      newcomer: collectUnhandledFailures(newcomer),
      owner: collectUnhandledFailures(owner),
    };

    await connectLiveSession(owner, {
      expectedDisplayName: 'Local Collaborator',
      password: ownerPassword ?? '',
      username: ownerUsername ?? '',
    });
    await waitUntilDeviceReady(owner);
    const catalogId = await createPrivateRoom(owner, name);
    await openConversation(owner);
    await say(owner, saidBefore ?? '');
    await makePublic(owner);

    await connectLiveSession(newcomer, {
      expectedDisplayName: 'Local Developer',
      password: newcomerPassword ?? '',
      username: newcomerUsername ?? '',
    });
    await waitUntilDeviceReady(newcomer);
    await joinFromExplore(newcomer, name, catalogId);
    await writeFile(joinedPath ?? '', `${JSON.stringify({ catalogId })}\n`, 'utf8');

    await waitForStep(agentInPath ?? '');
    await waitForAgentInRoster(owner, agentName ?? '');
    await openConversation(owner);
    await say(owner, saidAfter ?? '');

    await openConversation(newcomer);
    await expectSaid(newcomer, saidAfter ?? '');
    await expect(conversationLog(newcomer)).not.toContainText(saidBefore ?? '');

    // Agent 那边看完以前网页一直开着：房主的网页在线，真有补发钥匙的话这时候就发了。
    await waitForStep(checkedPath ?? '');
    await expect(conversationLog(newcomer)).not.toContainText(saidBefore ?? '');
    expect(failures).toEqual({ newcomer: [], owner: [] });
  } finally {
    await Promise.all(
      contexts.map(async (context) => {
        await context.close();
      }),
    );
  }
});
