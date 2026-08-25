import { isAbsolute } from 'node:path';
import { writeFile } from 'node:fs/promises';

import { expect, test } from '@playwright/test';
import { z } from 'zod';

import {
  apiOrigin,
  collectUnhandledFailures,
  connectLiveSession,
} from '../e2e-live/support/live-session';

const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const resultPath = process.env.AGENT_ROOM_VERTICAL_BOOTSTRAP_RESULT;
const creationRequestId = '019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a60';

const sessionSchema = z
  .object({
    matrixUserId: z.string().regex(/^@[^:]+:.+$/u),
    principalId: z.uuid(),
  })
  .loose();

const agentSchema = z
  .object({
    agentId: z.uuid(),
    displayName: z.literal('Vertical Codex Agent'),
    matrixUserId: z.string().regex(/^@[^:]+:.+$/u),
    slug: z.literal('vertical-codex-agent'),
  })
  .loose();

test('纵向切片使用真实登录幂等创建 Codex Agent', async ({ page }) => {
  test.skip(
    username === undefined || password === undefined || resultPath === undefined,
    '缺少纵向验收变量。',
  );
  expect(isAbsolute(resultPath ?? '')).toBe(true);
  const failures = collectUnhandledFailures(page);
  const matrixUserId = await connectLiveSession(page, username ?? '', password ?? '');

  const responses = await page.evaluate(
    async ({ apiBase, idempotencyKey }) => {
      const sessionResponse = await fetch(`${apiBase}/auth/session`, {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
      });
      const agentResponse = await fetch(`${apiBase}/agents`, {
        body: JSON.stringify({
          description: 'Task 24 real vertical slice agent',
          displayName: 'Vertical Codex Agent',
          slug: 'vertical-codex-agent',
          visibility: 'private',
        }),
        cache: 'no-store',
        credentials: 'include',
        headers: {
          Accept: 'application/json',
          'Content-Type': 'application/json',
          'Idempotency-Key': idempotencyKey,
        },
        method: 'POST',
      });
      return {
        agentBody: (await agentResponse.json()) as unknown,
        agentStatus: agentResponse.status,
        sessionBody: (await sessionResponse.json()) as unknown,
        sessionStatus: sessionResponse.status,
      };
    },
    { apiBase: apiOrigin, idempotencyKey: creationRequestId },
  );

  expect(responses.sessionStatus).toBe(200);
  expect(responses.agentStatus).toBe(201);
  const session = sessionSchema.parse(responses.sessionBody);
  const agent = agentSchema.parse(responses.agentBody);
  expect(session.matrixUserId).toBe(matrixUserId);

  await writeFile(
    resultPath ?? '',
    `${JSON.stringify(
      {
        agentId: agent.agentId,
        agentMatrixUserId: agent.matrixUserId,
        principalId: session.principalId,
        userMatrixUserId: session.matrixUserId,
      },
      null,
      2,
    )}\n`,
    'utf8',
  );
  expect(failures).toEqual([]);
});
