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
const resultPath = process.env.AGENT_ROOM_VERTICAL_KNOCK_RESULT;
const catalogId = process.env.AGENT_ROOM_VERTICAL_PRIVATE_ROOM_CATALOG_ID;
const agentId = process.env.AGENT_ROOM_VERTICAL_KNOCK_AGENT_ID;

const knocksSchema = z
  .object({
    knocks: z.array(z.object({ agentId: z.string(), displayName: z.string() }).loose()),
  })
  .loose();
const admittedSchema = z
  .object({ agent: z.object({ agentId: z.string(), status: z.literal('joined') }).loose() })
  .loose();

// 网络 Agent 已经拿房间号敲了门（specs/network-agents/knock.md）。房主用真实网页会话看在敲门的、
// 放它进来，和网页上点“让它进来”发的是同一个请求；服务器在这次请求里替它进端到端加密的房间。
// 不打开房间页：人的设备一进房间就会请别人补发之前的房间密钥，后面几轮要查网络 Agent 不请人补发。
test('房主用真实网页会话放敲门的网络 Agent 进私人房间', async ({ page }) => {
  test.skip(
    username === undefined ||
      password === undefined ||
      resultPath === undefined ||
      catalogId === undefined ||
      agentId === undefined,
    '缺少敲门纵向验收变量。',
  );
  expect(isAbsolute(resultPath ?? '')).toBe(true);
  const failures = collectUnhandledFailures(page);
  await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });

  const responses = await page.evaluate(
    async ({ apiBase, room, agent }) => {
      const base = `${apiBase}/private-rooms/${room}/agent-access`;
      const listed = await fetch(`${base}/knocks`, {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
        method: 'GET',
      });
      const knocks = { body: (await listed.json()) as unknown, status: listed.status };
      const admitted = await fetch(`${base}/agents/${agent}`, {
        cache: 'no-store',
        credentials: 'include',
        headers: { Accept: 'application/json' },
        method: 'PUT',
      });
      return {
        admitted: { body: (await admitted.json()) as unknown, status: admitted.status },
        knocks,
      };
    },
    { agent: agentId ?? '', apiBase: apiOrigin, room: catalogId ?? '' },
  );

  expect(responses.knocks.status).toBe(200);
  const { knocks } = knocksSchema.parse(responses.knocks.body);
  expect(knocks.map((knock) => knock.agentId)).toContain(agentId);
  expect(responses.admitted.status).toBe(200);
  const { agent } = admittedSchema.parse(responses.admitted.body);
  expect(agent.agentId).toBe(agentId);

  await writeFile(
    resultPath ?? '',
    `${JSON.stringify({ agentId: agent.agentId, status: agent.status }, null, 2)}\n`,
    'utf8',
  );
  expect(failures).toEqual([]);
});
