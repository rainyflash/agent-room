import { readFile, writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import { expect, test, type BrowserContext, type Page } from '@playwright/test';
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

// 须知与置顶（specs/redesign/brief-and-pins.md 第 4N-5 步）：房主在真实网页上建一间房和下面一间直接能进的
// 子房间，给上面那间生成 Agent 口令；纵向验收让他的本机 Agent 按“父房间名/子房间名”进子房间，另一个网络
// Agent 凭口令进上面那间、再进子房间。房主改上面那间的须知，两个 Agent 在子房间里都拿到；房主把本机 Agent
// 说的一句置顶，两个 Agent 都拿到，取消以后也都知道。网页和纵向验收靠几个文件对步子
// （tools/vertical.py 的 verify_brief_and_pins）。
const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const parentName = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_PARENT;
const subName = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_SUB;
const parentId = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_PARENT_ID;
const subId = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_SUB_ID;
const briefText = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_BRIEF;
const readyPath = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_READY;
const agentsInPath = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_AGENTS_IN;
const briefSetPath = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_BRIEF_SET;
const spokePath = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_SPOKE;
const pinnedPath = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_PINNED;
const pinsSeenPath = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_PINS_SEEN;
const unpinnedPath = process.env.AGENT_ROOM_VERTICAL_BRIEF_PINS_UNPINNED;

const stepPaths = [
  readyPath,
  agentsInPath,
  briefSetPath,
  spokePath,
  pinnedPath,
  pinsSeenPath,
  unpinnedPath,
];
const subRoomEntryMiss = /^HTTP 404 \S+\/lobbies\/([^/]+)\/entry$/u;
const setupSchema = z
  .object({
    parent: z.number(),
    sub: z.number(),
    code: z.object({ status: z.number(), body: z.unknown() }),
  })
  .strict();
const codeSchema = z
  .object({ code: z.string().regex(/^[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}$/u) })
  .loose();
const spokeSchema = z.object({ text: z.string().min(1) }).strict();

/**
 * 子房间不在房间列表里：打开 `/lobby/<子房间>` 时网页先走公开大厅入场，拿到“找不到”再单独问这一间
 * （`public-lobby-entry-boundary.tsx`），这一个 404 是设计如此。只放过这间子房间的，别的照旧算报错。
 */
function unexpected(failures: readonly string[]): string[] {
  return failures.filter((failure) => subRoomEntryMiss.exec(failure)?.[1] !== subId);
}

// 房间、子房间和口令用控制面的接口建（这一轮看的是须知和置顶，建房的界面别的验收看过了）。
async function setUp(owner: Page): Promise<string> {
  const answers = await owner.evaluate(
    async ({ apiBase, parent, sub, parentRoomName, subRoomName }) => {
      const post = (path: string, key: string, body: object) =>
        fetch(`${apiBase}${path}`, {
          body: JSON.stringify(body),
          cache: 'no-store',
          credentials: 'include',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
            'Idempotency-Key': key,
          },
          method: 'POST',
        });
      const madeParent = await post('/private-rooms', parent, { name: parentRoomName });
      const madeSub = await post(`/private-rooms/${parent}/sub-rooms`, sub, {
        name: subRoomName,
      });
      const code = await fetch(`${apiBase}/private-rooms/${parent}/agent-access/code`, {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
        method: 'PUT',
      });
      return {
        parent: madeParent.status,
        sub: madeSub.status,
        code: { status: code.status, body: (await code.json()) as unknown },
      };
    },
    {
      apiBase: apiOrigin,
      parent: parentId ?? '',
      sub: subId ?? '',
      parentRoomName: parentName ?? '',
      subRoomName: subName ?? '',
    },
  );
  const setup = setupSchema.parse(answers);
  expect(setup.parent).toBe(201);
  expect(setup.sub).toBe(201);
  expect(setup.code.status).toBe(200);
  return codeSchema.parse(setup.code.body).code;
}

// 打开一间房：网页替房主在聊天服务器上加入，Agent 说话时房间钥匙才发得到这台设备。
async function enter(owner: Page, catalogId: string): Promise<void> {
  await owner.goto(`/lobby/${catalogId}`);
  await expect(owner).toHaveURL(inRoom(catalogId), { timeout: 90_000 });
  await openConversation(owner);
  await expect(
    owner
      .getByRole('region', { name: 'Conversation', exact: true })
      .getByRole('textbox', { name: 'Message', exact: true }),
  ).toBeEnabled({ timeout: 120_000 });
}

async function writeBrief(owner: Page, text: string): Promise<void> {
  const settings = await openRoomSettings(owner, 'Brief');
  await settings.getByRole('textbox', { name: 'Brief', exact: true }).fill(text);
  await settings.getByRole('button', { name: 'Save brief', exact: true }).click();
  await expect(settings.getByText('Brief updated.', { exact: true })).toBeVisible({
    timeout: 60_000,
  });
  await closeRoomSettings(owner, settings);
}

function pinsBar(owner: Page) {
  return owner.getByRole('region', { name: 'Pinned messages', exact: true });
}

test.use({ actionTimeout: 90_000 });

test('须知与置顶：父房间的须知和子房间的置顶，子房间里的本机 Agent 和网络 Agent 都拿到', async ({
  browser,
}) => {
  test.skip(
    username === undefined ||
      password === undefined ||
      parentName === undefined ||
      subName === undefined ||
      parentId === undefined ||
      subId === undefined ||
      briefText === undefined ||
      stepPaths.some((path) => path === undefined),
    '缺少须知与置顶纵向验收变量。',
  );
  test.setTimeout(1_800_000);
  for (const path of stepPaths) expect(isAbsolute(path ?? '')).toBe(true);

  const contexts: BrowserContext[] = [];
  try {
    const owner = await openIsolatedPage(browser, contexts);
    const failures = collectUnhandledFailures(owner);
    await connectLiveSession(owner, {
      expectedDisplayName: 'Local Developer',
      password: password ?? '',
      username: username ?? '',
    });
    await waitUntilDeviceReady(owner);
    const code = await setUp(owner);
    await enter(owner, parentId ?? '');
    await enter(owner, subId ?? '');
    await writeFile(readyPath ?? '', `${JSON.stringify({ code })}\n`, 'utf8');

    // 两个 Agent 都进了子房间，房主改上面那间的须知；子房间页上先写着上面那间的。
    await waitForStep(agentsInPath ?? '');
    await enter(owner, parentId ?? '');
    await writeBrief(owner, briefText ?? '');
    await enter(owner, subId ?? '');
    await expect(
      owner.getByRole('region', { name: `Brief from ${parentName ?? ''}`, exact: true }),
    ).toContainText(briefText ?? '', { timeout: 120_000 });
    await writeFile(briefSetPath ?? '', '{}\n', 'utf8');

    // 本机 Agent 在子房间说了一句，房主把它置顶。
    await waitForStep(spokePath ?? '');
    const spoke = spokeSchema.parse(JSON.parse(await readFile(spokePath ?? '', 'utf8')));
    await enter(owner, subId ?? '');
    await expectSaid(owner, spoke.text);
    const message = conversationLog(owner)
      .locator('[data-conversation-message-id]')
      .filter({ hasText: spoke.text });
    await message.hover();
    await message.getByRole('button', { name: 'Pin this message', exact: true }).click();
    await expect(pinsBar(owner).locator('.room-pins__count')).toHaveText('1', {
      timeout: 60_000,
    });
    await expect(pinsBar(owner).locator('.room-pins__summary')).toContainText(spoke.text);
    await writeFile(pinnedPath ?? '', '{}\n', 'utf8');

    // 两个 Agent 都拿到这条置顶以后，房主在置顶那一行里取消它。
    await waitForStep(pinsSeenPath ?? '');
    await pinsBar(owner).locator('.room-pins__toggle').click();
    await pinsBar(owner)
      .getByRole('listitem')
      .first()
      .getByRole('button', { name: 'Unpin this message', exact: true })
      .click();
    await expect(pinsBar(owner)).toHaveCount(0, { timeout: 60_000 });
    await expect(conversationLog(owner).locator('.room-notice-change').last()).toContainText(
      'You unpinned a message',
    );
    expect(unexpected(failures)).toEqual([]);
    await writeFile(unpinnedPath ?? '', '{}\n', 'utf8');
  } finally {
    await Promise.all(
      contexts.map(async (context) => {
        await context.close();
      }),
    );
  }
});
