import { writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import { expect, test } from '@playwright/test';
import { z } from 'zod';

import {
  apiOrigin,
  collectUnhandledFailures,
  connectLiveSession,
} from '../e2e-live/support/live-session';

const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const resultPath = process.env.AGENT_ROOM_VERTICAL_PRIVATE_ROOM_RESULT;
const catalogId = process.env.AGENT_ROOM_VERTICAL_PRIVATE_ROOM_CATALOG_ID;

const roomSchema = z
  .object({
    catalogId: z.string(),
    matrixRoomId: z.string().startsWith('!'),
  })
  .loose();
const codeSchema = z
  .object({
    code: z.string().regex(/^[A-Z0-9]{4}-[A-Z0-9]{4}-[A-Z0-9]{4}$/u),
  })
  .loose();

// 房间由服务器建好（端到端加密），人不必进 Matrix 房间，Agent 凭口令进来就能说话。
test('真实浏览器会话建私人房间并生成 Agent 口令', async ({ page }) => {
  test.skip(
    username === undefined ||
      password === undefined ||
      resultPath === undefined ||
      catalogId === undefined,
    '缺少私人房间纵向验收变量。',
  );
  expect(isAbsolute(resultPath ?? '')).toBe(true);
  const failures = collectUnhandledFailures(page);
  await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });

  const responses = await page.evaluate(
    async ({ apiBase, id }) => {
      const created = await fetch(`${apiBase}/private-rooms`, {
        body: JSON.stringify({ name: 'Vertical Private Room' }),
        cache: 'no-store',
        credentials: 'include',
        headers: {
          Accept: 'application/json',
          'Content-Type': 'application/json',
          'Idempotency-Key': id,
        },
        method: 'POST',
      });
      const room = { body: (await created.json()) as unknown, status: created.status };
      const generated = await fetch(`${apiBase}/private-rooms/${id}/agent-access/code`, {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
        method: 'PUT',
      });
      return {
        code: { body: (await generated.json()) as unknown, status: generated.status },
        room,
      };
    },
    { apiBase: apiOrigin, id: catalogId ?? '' },
  );

  expect(responses.room.status).toBe(201);
  expect(responses.code.status).toBe(200);
  const room = roomSchema.parse(responses.room.body);
  expect(room.catalogId).toBe(catalogId);
  const { code } = codeSchema.parse(responses.code.body);

  await writeFile(
    resultPath ?? '',
    `${JSON.stringify({ catalogId: room.catalogId, code, matrixRoomId: room.matrixRoomId }, null, 2)}\n`,
    'utf8',
  );
  // 登录后自动签好这台设备（ADR 0011）。账户已有签名身份、服务器上却没有钥匙时走“重建签名身份”，
  // 新签名公钥由控制面以应用服务的身份代传；不管走哪条路，最后都要是“已就绪”。
  await page.goto('/settings/security');
  await expect(page.getByText(/^(?:This device is ready|这台设备已就绪)$/u)).toBeVisible({
    timeout: 60_000,
  });
  expect(failures).toEqual([]);
});
