import { writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import { expect, test } from '@playwright/test';

import { collectUnhandledFailures, connectLiveSession } from '../e2e-live/support/live-session';

const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const catalogId = process.env.AGENT_ROOM_VERTICAL_LOCAL_ROOM_CATALOG_ID;
const roomName = process.env.AGENT_ROOM_VERTICAL_LOCAL_ROOM_NAME;
const greeting = process.env.AGENT_ROOM_VERTICAL_LOCAL_ROOM_GREETING;
const said = process.env.AGENT_ROOM_VERTICAL_LOCAL_ROOM_SAID;
const enteredPath = process.env.AGENT_ROOM_VERTICAL_LOCAL_ROOM_ENTERED;
const spokePath = process.env.AGENT_ROOM_VERTICAL_LOCAL_ROOM_SPOKE;

// 本机 Agent 替验收账号建了私人房间（specs/agent-rooms/design.md 验收第 1 条），房主在 Matrix 上还只是受邀。
// 人在“房间”页点这个房间就进去：网页替他补上加入，不叫他回列表找“加入”。进去以后写下 entered 文件，
// 建房的 Agent 说一句；人解开以后再说一句，写下 spoke 文件，纵向验收看 Agent 解开收到。
test('房主在“房间”页点一下就进了本机 Agent 替他建的房间，看得到它说的话，也能说话', async ({
  page,
}) => {
  test.skip(
    username === undefined ||
      password === undefined ||
      catalogId === undefined ||
      roomName === undefined ||
      greeting === undefined ||
      said === undefined ||
      enteredPath === undefined ||
      spokePath === undefined,
    '缺少本机 Agent 房间纵向验收变量。',
  );
  test.setTimeout(420_000);
  expect(isAbsolute(enteredPath ?? '')).toBe(true);
  expect(isAbsolute(spokePath ?? '')).toBe(true);
  const failures = collectUnhandledFailures(page);
  await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });

  // 服务器记着房主已经在里面：列在“你的私人房间”里，不在邀请里，也没有“加入”按钮。
  const card = page
    .getByRole('region', { name: 'Your private rooms', exact: true })
    .getByRole('link')
    .filter({ hasText: roomName ?? '' });
  await expect(card).toBeVisible({ timeout: 60_000 });
  await card.click();
  await expect(page).toHaveURL(new RegExp(`/lobby/${catalogId ?? ''}/instance/`, 'u'), {
    timeout: 30_000,
  });
  await expect(page.locator('.workspace-header__identity > span')).toHaveText('Private room', {
    timeout: 90_000,
  });
  await expect(page.getByText('You are not in this room yet')).toHaveCount(0);

  const room = new URL(page.url());
  room.searchParams.set('view', 'conversation');
  await page.goto(`${room.pathname}${room.search}`);
  const conversation = page.getByRole('region', { name: 'Conversation', exact: true });
  await expect(conversation.getByRole('textbox', { name: 'Message', exact: true })).toBeEnabled({
    timeout: 60_000,
  });
  await writeFile(enteredPath ?? '', `${JSON.stringify({ entered: true })}\n`, 'utf8');
  await expect(conversation.getByRole('log')).toContainText(greeting ?? '', { timeout: 120_000 });
  await conversation.getByRole('textbox', { name: 'Message', exact: true }).fill(said ?? '');
  await conversation.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(conversation.getByRole('log')).toContainText(said ?? '', { timeout: 60_000 });
  expect(failures).toEqual([]);
  await writeFile(spokePath ?? '', `${JSON.stringify({ spoke: true })}\n`, 'utf8');
});
