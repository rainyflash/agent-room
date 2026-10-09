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
const resultPath = process.env.AGENT_ROOM_VERTICAL_WATCH_RESULT;
const slug = process.env.AGENT_ROOM_VERTICAL_WATCH_SLUG;
const catalogId = process.env.AGENT_ROOM_VERTICAL_WATCH_CATALOG_ID;
const probe = process.env.AGENT_ROOM_VERTICAL_WATCH_PROBE;
const eventId = process.env.AGENT_ROOM_VERTICAL_WATCH_EVENT_ID;
const actionId = process.env.AGENT_ROOM_VERTICAL_WATCH_ACTION_ID;

const actionSchema = z
  .object({ actionId: z.string(), kind: z.literal('hide'), status: z.string() })
  .loose();

// 不登录看公共大厅（specs/public-lobby-watch/design.md 第 3 步）。网络 Agent 已经在公共大厅说了一句话：
// 没登录的访客在网页上看得到；平台管理员用网页上隐藏消息时发的同一个请求把它隐藏，访客的页面跟着拿掉。
test('不登录在网页上看公共大厅，管理员隐藏以后那句话不见了', async ({ browser }) => {
  test.skip(
    username === undefined ||
      password === undefined ||
      resultPath === undefined ||
      slug === undefined ||
      catalogId === undefined ||
      probe === undefined ||
      eventId === undefined ||
      actionId === undefined,
    '缺少围观纵向验收变量。',
  );
  expect(isAbsolute(resultPath ?? '')).toBe(true);

  const visitorContext = await browser.newContext();
  const visitor = await visitorContext.newPage();
  const visitorFailures = collectUnhandledFailures(visitor);
  await visitor.goto(`/watch/${slug ?? ''}`);
  const said = visitor.getByRole('log').getByText(probe ?? '', { exact: true });
  await expect(said).toBeVisible({ timeout: 30_000 });
  await expect(visitor.locator('meta[name="robots"]')).toHaveAttribute('content', 'noindex');

  const moderatorContext = await browser.newContext();
  const moderator = await moderatorContext.newPage();
  const moderatorFailures = collectUnhandledFailures(moderator);
  await connectLiveSession(moderator, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });
  const hidden = await moderator.evaluate(
    async ({ action, apiBase, event, room }) => {
      const response = await fetch(`${apiBase}/rooms/${room}/moderation/actions`, {
        body: JSON.stringify({
          impactAcknowledged: true,
          kind: 'hide',
          reason: 'spam',
          targetKind: 'event',
          targetReference: event,
        }),
        cache: 'no-store',
        credentials: 'include',
        headers: {
          Accept: 'application/json',
          'Content-Type': 'application/json',
          'Idempotency-Key': action,
        },
        method: 'POST',
      });
      return { body: (await response.json()) as unknown, status: response.status };
    },
    { action: actionId ?? '', apiBase: apiOrigin, event: eventId ?? '', room: catalogId ?? '' },
  );
  expect(hidden.status).toBe(201);
  const action = actionSchema.parse(hidden.body);
  expect(action.actionId).toBe(actionId);

  // 访客的页面 5 秒问一次，服务器的快照 3 秒一换：隐藏以后很快就拿掉。
  await expect(said).toHaveCount(0, { timeout: 30_000 });

  await writeFile(
    resultPath ?? '',
    `${JSON.stringify({ actionId: action.actionId, actionStatus: action.status, visitor: 'saw-then-lost' })}\n`,
    'utf8',
  );
  expect(visitorFailures).toEqual([]);
  expect(moderatorFailures).toEqual([]);
  await visitorContext.close();
  await moderatorContext.close();
});
