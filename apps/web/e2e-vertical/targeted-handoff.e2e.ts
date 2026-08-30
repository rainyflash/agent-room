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
const resultPath = process.env.AGENT_ROOM_VERTICAL_HANDOFF_RESULT;

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu);
const targetListSchema = z
  .object({
    targets: z.array(
      z
        .object({
          agentInstanceId: uuidV7Schema,
        })
        .loose(),
    ),
  })
  .strict();
const handoffSchema = z
  .object({
    created: z.boolean(),
    handoffId: uuidV7Schema,
    status: z.enum(['queued', 'delivered', 'consumed', 'declined', 'revoked', 'expired', 'failed']),
    target: z
      .object({
        agentInstanceId: uuidV7Schema,
      })
      .loose(),
  })
  .loose();
const input = readInput(process.env);

test('真实浏览器会话创建并幂等重放实例定向交接', async ({ page }) => {
  test.skip(
    username === undefined || password === undefined || resultPath === undefined || input === null,
    '缺少定向交接纵向验收变量。',
  );
  expect(isAbsolute(resultPath ?? '')).toBe(true);
  const request = input ?? missingInput();
  const failures = collectUnhandledFailures(page);
  await connectLiveSession(page, username ?? '', password ?? '');
  await page.goto(`/lobby/${request.catalogId}`);
  await expect
    .poll(() => new URL(page.url()).pathname, { timeout: 40_000 })
    .toContain(`/lobby/${request.catalogId}/instance/`);
  expect(decodeURIComponent(new URL(page.url()).pathname)).toContain(request.sourceRoomId);

  const responses = await page.evaluate(
    async ({ apiBase, handoff }) => {
      const targetsEndpoint = new URL(`${apiBase}/handoff-targets`);
      targetsEndpoint.searchParams.set('roomId', handoff.sourceRoomId);
      const targetsResponse = await fetch(targetsEndpoint, {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
      });
      const body = JSON.stringify({
        contentId: handoff.contentId,
        expiresAtUnixMs: handoff.expiresAtUnixMs,
        permissions: ['read_text', 'include_metadata'],
        purpose: 'summarize',
        sourceEventId: handoff.sourceEventId,
        sourceMessageId: handoff.sourceMessageId,
        sourceRoomId: handoff.sourceRoomId,
        targetInstanceId: handoff.targetInstanceId,
      });
      const submit = async () => {
        const response = await fetch(`${apiBase}/handoffs`, {
          body,
          cache: 'no-store',
          credentials: 'include',
          headers: {
            Accept: 'application/json',
            'Content-Type': 'application/json',
            'Idempotency-Key': handoff.handoffId,
          },
          method: 'POST',
        });
        return {
          body: (await response.json()) as unknown,
          status: response.status,
        };
      };
      return {
        first: await submit(),
        replay: await submit(),
        targetsBody: (await targetsResponse.json()) as unknown,
        targetsStatus: targetsResponse.status,
      };
    },
    { apiBase: apiOrigin, handoff: request },
  );

  expect(responses.targetsStatus).toBe(200);
  const targets = targetListSchema.parse(responses.targetsBody);
  expect(
    targets.targets.some((target) => target.agentInstanceId === request.targetInstanceId),
  ).toBe(true);
  expect(responses.first.status).toBe(201);
  expect(responses.replay.status).toBe(200);
  const first = handoffSchema.parse(responses.first.body);
  const replay = handoffSchema.parse(responses.replay.body);
  expect(first.handoffId).toBe(request.handoffId);
  expect(first.target.agentInstanceId).toBe(request.targetInstanceId);
  expect(first.created).toBe(true);
  expect(replay).toMatchObject({
    created: false,
    handoffId: first.handoffId,
  });

  await writeFile(
    resultPath ?? '',
    `${JSON.stringify(
      {
        handoffId: first.handoffId,
        replayed: String(!replay.created),
        status: first.status,
        targetInstanceId: first.target.agentInstanceId,
      },
      null,
      2,
    )}\n`,
    'utf8',
  );
  expect(failures).toEqual([]);
});

type HandoffInput = Readonly<{
  catalogId: string;
  contentId: string;
  expiresAtUnixMs: number;
  handoffId: string;
  sourceEventId: string;
  sourceMessageId: string;
  sourceRoomId: string;
  targetInstanceId: string;
}>;

function readInput(environment: NodeJS.ProcessEnv): HandoffInput | null {
  const values = {
    catalogId: environment.AGENT_ROOM_VERTICAL_HANDOFF_CATALOG_ID,
    contentId: environment.AGENT_ROOM_VERTICAL_HANDOFF_CONTENT_ID,
    expiresAtUnixMs: environment.AGENT_ROOM_VERTICAL_HANDOFF_EXPIRES_AT_UNIX_MS,
    handoffId: environment.AGENT_ROOM_VERTICAL_HANDOFF_ID,
    sourceEventId: environment.AGENT_ROOM_VERTICAL_HANDOFF_SOURCE_EVENT_ID,
    sourceMessageId: environment.AGENT_ROOM_VERTICAL_HANDOFF_SOURCE_MESSAGE_ID,
    sourceRoomId: environment.AGENT_ROOM_VERTICAL_HANDOFF_SOURCE_ROOM_ID,
    targetInstanceId: environment.AGENT_ROOM_VERTICAL_HANDOFF_TARGET_INSTANCE_ID,
  };
  if (Object.values(values).some((value) => value === undefined)) {
    return null;
  }
  return z
    .object({
      catalogId: uuidV7Schema,
      contentId: uuidV7Schema,
      expiresAtUnixMs: z.coerce.number().int().positive(),
      handoffId: uuidV7Schema,
      sourceEventId: z.string().min(4).max(1_024).startsWith('$'),
      sourceMessageId: uuidV7Schema,
      sourceRoomId: z.string().min(4).max(512).startsWith('!'),
      targetInstanceId: uuidV7Schema,
    })
    .parse(values);
}

function missingInput(): HandoffInput {
  throw new Error('定向交接纵向验收输入缺失。');
}
