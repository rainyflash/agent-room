import { writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import { expect, test, type Page } from '@playwright/test';
import { z } from 'zod';

import {
  apiOrigin,
  collectUnhandledFailures,
  connectLiveSession,
} from '../e2e-live/support/live-session';

const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const step = process.env.AGENT_ROOM_VERTICAL_AGENT_IDENTITY_STEP;
const resultPath = process.env.AGENT_ROOM_VERTICAL_AGENT_IDENTITY_RESULT;
const roomsToCreate = process.env.AGENT_ROOM_VERTICAL_AGENT_IDENTITY_ROOMS;
const agentId = process.env.AGENT_ROOM_VERTICAL_AGENT_IDENTITY_AGENT;
const catalogId = process.env.AGENT_ROOM_VERTICAL_AGENT_IDENTITY_CATALOG;

type Answer = { readonly status: number; readonly body: unknown };

const roomsSchema = z.array(z.object({ catalogId: z.string(), name: z.string() }).strict());
const createdSchema = z
  .object({ catalogId: z.string(), matrixRoomId: z.string().startsWith('!') })
  .loose();
const agentRoomsSchema = z
  .object({ rooms: z.array(z.object({ catalogId: z.string() }).loose()) })
  .strict();
const instancesSchema = z
  .object({
    instances: z.array(
      z
        .object({
          agentId: z.string(),
          agentInstanceId: z.string(),
          revokedAtUnixMs: z.number().nullable(),
        })
        .loose(),
    ),
  })
  .loose();

/** 页面里带着登录发一个控制面请求，回答状态和正文（没有正文时为 null）。 */
async function request(
  page: Page,
  path: string,
  method: string,
  body?: unknown,
  extraHeaders: Record<string, string> = {},
): Promise<Answer> {
  return await page.evaluate(
    async ({ base, target, verb, payload, headers }) => {
      const response = await fetch(`${base}${target}`, {
        cache: 'no-store',
        credentials: 'include',
        headers: {
          Accept: 'application/json',
          ...(payload === null ? {} : { 'Content-Type': 'application/json' }),
          ...headers,
        },
        method: verb,
        ...(payload === null ? {} : { body: JSON.stringify(payload) }),
      });
      const text = await response.text();
      return { body: text === '' ? null : (JSON.parse(text) as unknown), status: response.status };
    },
    { base: apiOrigin, target: path, verb: method, payload: body ?? null, headers: extraHeaders },
  );
}

async function createRooms(page: Page): Promise<unknown> {
  const wanted = roomsSchema.parse(JSON.parse(roomsToCreate ?? '[]'));
  const rooms = [];
  for (const room of wanted) {
    const created = await request(
      page,
      '/private-rooms',
      'POST',
      { name: room.name },
      { 'Idempotency-Key': room.catalogId },
    );
    expect(created.status).toBe(201);
    const parsed = createdSchema.parse(created.body);
    expect(parsed.catalogId).toBe(room.catalogId);
    rooms.push({ catalogId: parsed.catalogId, matrixRoomId: parsed.matrixRoomId, name: room.name });
  }
  return { rooms };
}

async function agentRooms(page: Page): Promise<string[]> {
  const listed = await request(page, `/agents/${agentId ?? ''}/rooms`, 'GET');
  expect(listed.status).toBe(200);
  return agentRoomsSchema.parse(listed.body).rooms.map((room) => room.catalogId);
}

/** 主人从“我的 Agent”带它进房间：服务器替它这台电脑上的连接进去。 */
async function bring(page: Page): Promise<unknown> {
  const brought = await request(page, `/agents/${agentId ?? ''}/rooms`, 'POST', { catalogId });
  expect(brought.status).toBe(200);
  return { brought: brought.body, rooms: await agentRooms(page) };
}

/** 主人让它离开房间，再删掉它：删除先撤掉它的每个连接，和网页上的做法一样。 */
async function leaveAndDelete(page: Page): Promise<unknown> {
  const left = await request(page, `/agents/${agentId ?? ''}/rooms/${catalogId ?? ''}`, 'DELETE');
  expect(left.status).toBe(200);
  const roomsAfterLeave = await agentRooms(page);
  const listed = await request(page, '/agent-instances', 'GET');
  expect(listed.status).toBe(200);
  const live = instancesSchema
    .parse(listed.body)
    .instances.filter((entry) => entry.agentId === agentId && entry.revokedAtUnixMs === null);
  for (const entry of live) {
    const revoked = await request(page, `/agent-instances/${entry.agentInstanceId}`, 'DELETE');
    expect(revoked.status).toBeLessThan(300);
  }
  const deleted = await request(page, `/agents/${agentId ?? ''}`, 'DELETE');
  return { left: left.body, roomsAfterLeave, revoked: live.length, deleted: deleted.status };
}

// 本机 Agent 跨房间用同一个身份那一轮（specs/agent-identity/design.md“验收”）里主人做的事。
test('主人建房间、带 Agent 进房间、让它离开、删掉它', async ({ page }) => {
  test.skip(
    username === undefined || password === undefined || resultPath === undefined,
    '缺少同一个身份那一轮的纵向验收变量。',
  );
  expect(isAbsolute(resultPath ?? '')).toBe(true);
  const failures = collectUnhandledFailures(page);
  await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });
  const result =
    step === 'rooms'
      ? await createRooms(page)
      : step === 'bring'
        ? await bring(page)
        : step === 'leave-and-delete'
          ? await leaveAndDelete(page)
          : null;
  expect(result).not.toBeNull();
  await writeFile(resultPath ?? '', `${JSON.stringify(result, null, 2)}\n`, 'utf8');
  expect(failures).toEqual([]);
});
