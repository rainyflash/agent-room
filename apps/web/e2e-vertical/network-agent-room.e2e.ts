import { access, writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import { expect, test } from '@playwright/test';

import { collectUnhandledFailures, connectLiveSession } from '../e2e-live/support/live-session';

const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const catalogId = process.env.AGENT_ROOM_VERTICAL_AGENT_ROOM_CATALOG_ID;
const roomName = process.env.AGENT_ROOM_VERTICAL_AGENT_ROOM_NAME;
const said = process.env.AGENT_ROOM_VERTICAL_AGENT_ROOM_SAID;
const reply = process.env.AGENT_ROOM_VERTICAL_AGENT_ROOM_REPLY;
const chattedPath = process.env.AGENT_ROOM_VERTICAL_AGENT_ROOM_CHATTED;
const archivedPath = process.env.AGENT_ROOM_VERTICAL_AGENT_ROOM_ARCHIVED;

async function exists(path: string): Promise<boolean> {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

// 网络 Agent 建了私人房间、按账户 ID 邀请了验收账号（specs/agent-rooms/design.md 验收第 4、6 条）。
// 人在“房间”页看到“服务器能读到”的提醒后接受，进房间说一句、等房主回；纵向验收在房主离开、房间
// 归档以后写下 archived 文件，同一个网页会话重新打开房间，之前的消息照样看得到。用同一个会话，
// 不用另开设备从密钥备份里找回。
test('人接受网络 Agent 的邀请，在它建的房间里聊天，房间归档后还看得到之前的消息', async ({
  page,
}) => {
  test.skip(
    username === undefined ||
      password === undefined ||
      catalogId === undefined ||
      roomName === undefined ||
      said === undefined ||
      reply === undefined ||
      chattedPath === undefined ||
      archivedPath === undefined,
    '缺少网络 Agent 房间纵向验收变量。',
  );
  test.setTimeout(600_000);
  expect(isAbsolute(chattedPath ?? '')).toBe(true);
  expect(isAbsolute(archivedPath ?? '')).toBe(true);
  const failures = collectUnhandledFailures(page);
  await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });

  const invitation = page
    .getByRole('list', { name: 'Invitations', exact: true })
    .getByRole('listitem')
    .filter({ hasText: roomName ?? '' });
  await expect(invitation).toContainText(
    'A network agent made this room. The server can read what is said here.',
    { timeout: 60_000 },
  );
  await invitation.getByRole('button', { name: 'Join', exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`/lobby/${catalogId ?? ''}/instance/`, 'u'), {
    timeout: 90_000,
  });

  const room = new URL(page.url());
  room.searchParams.set('view', 'conversation');
  await page.goto(`${room.pathname}${room.search}`);
  const conversation = page.getByRole('region', { name: 'Conversation', exact: true });
  await conversation.getByRole('textbox', { name: 'Message', exact: true }).fill(said ?? '');
  await conversation.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(conversation.getByRole('log')).toContainText(said ?? '', { timeout: 60_000 });
  await expect(conversation.getByRole('log')).toContainText(reply ?? '', { timeout: 120_000 });
  expect(failures).toEqual([]);
  await writeFile(chattedPath ?? '', `${JSON.stringify({ chatted: true })}\n`, 'utf8');

  // 归档以后只看之前的消息还在不在，不再收集网页的报错。
  await expect
    .poll(async () => await exists(archivedPath ?? ''), { intervals: [1_000], timeout: 420_000 })
    .toBe(true);
  await page.reload();
  const reopened = page.getByRole('region', { name: 'Conversation', exact: true }).getByRole('log');
  await expect(reopened).toContainText(said ?? '', { timeout: 90_000 });
  await expect(reopened).toContainText(reply ?? '', { timeout: 90_000 });
});
