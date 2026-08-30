import { writeFile } from 'node:fs/promises';
import { isAbsolute } from 'node:path';

import {
  expect,
  test,
  type Browser,
  type BrowserContext,
  type Page,
  type Response,
} from '@playwright/test';
import { z } from 'zod';

import {
  apiOrigin,
  collectUnhandledFailures,
  connectLiveSession,
  readMatrixSession,
  type LiveSessionCredentials,
} from '../e2e-live/support/live-session';

const applicationOrigin = 'https://app.agent-room.localhost:18443';
const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu);
const matrixUserIdSchema = z.string().regex(/^@[^:]+:.+$/u);
const matrixSessionSchema = z
  .object({
    deviceId: z.string().min(1),
    userId: matrixUserIdSchema,
  })
  .loose();
const controlPlaneSessionSchema = z
  .object({
    displayName: z.string().min(1),
    matrixUserId: matrixUserIdSchema,
    principalId: uuidV7Schema,
  })
  .loose();
const handoffResponseSchema = z
  .object({
    content: z
      .object({
        contentId: uuidV7Schema,
      })
      .loose(),
    handoffId: uuidV7Schema,
    principalId: uuidV7Schema,
    source: z
      .object({
        matrixEventId: z.string().startsWith('$'),
        matrixRoomId: z.string().startsWith('!'),
        messageId: uuidV7Schema,
      })
      .loose(),
    status: z.enum(['queued', 'delivered']),
    target: z
      .object({
        agentInstanceId: uuidV7Schema,
      })
      .loose(),
  })
  .loose();

const input = readInput(process.env);

test('三套独立浏览器会话完成云端工作区、跨账户消息与定向交接', async ({ browser }) => {
  test.setTimeout(360_000);
  test.skip(input === null, '缺少产品闭环纵向验收变量。');
  const scenario = input ?? missingInput();
  expect(isAbsolute(scenario.resultPath)).toBe(true);

  const contexts: BrowserContext[] = [];
  try {
    const ownerPrimary = await openIsolatedPage(browser, contexts);
    const ownerSecondary = await openIsolatedPage(browser, contexts);
    const collaborator = await openIsolatedPage(browser, contexts);
    const failures = [
      ...collectUnhandledFailures(ownerPrimary),
      ...collectUnhandledFailures(ownerSecondary),
      ...collectUnhandledFailures(collaborator),
    ];

    const ownerCredentials: LiveSessionCredentials = {
      expectedDisplayName: 'Local Developer',
      password: scenario.ownerPassword,
      username: scenario.ownerUsername,
    };
    const collaboratorCredentials: LiveSessionCredentials = {
      expectedDisplayName: 'Local Collaborator',
      password: scenario.collaboratorPassword,
      username: scenario.collaboratorUsername,
    };
    const [ownerPrimaryUserId, ownerSecondaryUserId, collaboratorUserId] = await Promise.all([
      connectLiveSession(ownerPrimary, ownerCredentials),
      connectLiveSession(ownerSecondary, ownerCredentials),
      connectLiveSession(collaborator, collaboratorCredentials),
    ]);
    expect(ownerSecondaryUserId).toBe(ownerPrimaryUserId);
    expect(collaboratorUserId).not.toBe(ownerPrimaryUserId);

    const ownerPrimaryMatrix = matrixSessionSchema.parse(await readMatrixSession(ownerPrimary));
    const ownerSecondaryMatrix = matrixSessionSchema.parse(await readMatrixSession(ownerSecondary));
    expect(ownerSecondaryMatrix.deviceId).not.toBe(ownerPrimaryMatrix.deviceId);

    const collaboratorSession = await readControlPlaneSession(collaborator);
    expect(collaboratorSession.displayName).toBe('Local Collaborator');
    expect(collaboratorSession.matrixUserId).toBe(collaboratorUserId);
    await grantAgentMembership(ownerPrimary, scenario.agentId, collaboratorSession.principalId);

    await Promise.all([
      verifyCloudWorkspace(ownerPrimary, 'Local Developer'),
      verifyCloudWorkspace(ownerSecondary, 'Local Developer'),
      verifyCloudWorkspace(collaborator, 'Local Collaborator'),
    ]);

    const [ownerPrimaryRoomId, ownerSecondaryRoomId, collaboratorRoomId] = await Promise.all([
      enterPublicLobby(ownerPrimary, scenario.catalogId),
      enterPublicLobby(ownerSecondary, scenario.catalogId),
      enterPublicLobby(collaborator, scenario.catalogId),
    ]);
    expect(ownerSecondaryRoomId).toBe(ownerPrimaryRoomId);
    expect(collaboratorRoomId).toBe(ownerPrimaryRoomId);

    const messageTitle = `Collaborator closure ${Date.now().toString(36)}`;
    const messageBody = `Second Agent Room account verified cloud delivery at ${new Date().toISOString()}.`;
    await publishHumanMessage(collaborator, messageTitle, messageBody);
    await verifyRemoteMessage(ownerPrimary, messageTitle, messageBody);
    await expect(ownerSecondary.locator('.message-dock__headline')).toContainText(messageTitle, {
      timeout: 45_000,
    });

    const handoffResponse = await createTargetedHandoffFromMessage(
      collaborator,
      messageTitle,
      messageBody,
      scenario.targetInstanceId,
    );
    expect(handoffResponse.principalId).toBe(collaboratorSession.principalId);
    expect(handoffResponse.source.matrixRoomId).toBe(collaboratorRoomId);
    expect(handoffResponse.target.agentInstanceId).toBe(scenario.targetInstanceId);

    await writeFile(
      scenario.resultPath,
      `${JSON.stringify(
        {
          browserContextCount: '3',
          collaboratorMatrixUserId: collaboratorUserId,
          collaboratorPrincipalId: collaboratorSession.principalId,
          contentId: handoffResponse.content.contentId,
          handoffId: handoffResponse.handoffId,
          messageBody,
          messageId: handoffResponse.source.messageId,
          ownerMatrixDeviceCount: '2',
          ownerMatrixUserId: ownerPrimaryUserId,
          roomId: collaboratorRoomId,
          sourceEventId: handoffResponse.source.matrixEventId,
          targetInstanceId: handoffResponse.target.agentInstanceId,
        },
        null,
        2,
      )}\n`,
      'utf8',
    );
    expect(failures).toEqual([]);
  } finally {
    await Promise.all(
      contexts.map(async (context) => {
        await context.close();
      }),
    );
  }
});

type ProductClosureInput = Readonly<{
  agentId: string;
  catalogId: string;
  collaboratorPassword: string;
  collaboratorUsername: string;
  ownerPassword: string;
  ownerUsername: string;
  resultPath: string;
  targetInstanceId: string;
}>;

async function openIsolatedPage(browser: Browser, contexts: BrowserContext[]): Promise<Page> {
  const context = await browser.newContext({
    baseURL: applicationOrigin,
    ignoreHTTPSErrors: true,
    locale: 'en-US',
  });
  contexts.push(context);
  return await context.newPage();
}

async function readControlPlaneSession(page: Page) {
  const response = await page.evaluate(async (apiBase) => {
    const session = await fetch(`${apiBase}/auth/session`, {
      cache: 'no-store',
      credentials: 'include',
      headers: { Accept: 'application/json' },
    });
    return {
      body: (await session.json()) as unknown,
      status: session.status,
    };
  }, apiOrigin);
  expect(response.status).toBe(200);
  return controlPlaneSessionSchema.parse(response.body);
}

async function grantAgentMembership(
  owner: Page,
  agentId: string,
  collaboratorPrincipalId: string,
): Promise<void> {
  const status = await owner.evaluate(
    async ({ apiBase, principalId, sharedAgentId }) => {
      const response = await fetch(
        `${apiBase}/agents/${encodeURIComponent(sharedAgentId)}/members/${encodeURIComponent(principalId)}`,
        {
          body: JSON.stringify({ role: 'operator' }),
          cache: 'no-store',
          credentials: 'include',
          headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
          method: 'PUT',
        },
      );
      return response.status;
    },
    { apiBase: apiOrigin, principalId: collaboratorPrincipalId, sharedAgentId: agentId },
  );
  expect(status).toBe(204);
}

async function verifyCloudWorkspace(page: Page, expectedDisplayName: string): Promise<void> {
  await page.goto('/workspace');
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Every Agent\. Every device\. One account truth\.|所有 Agent、所有设备，共用一个账户事实。/u,
    { timeout: 40_000 },
  );
  await expect(page.locator('.account-workspace__intro')).toContainText(expectedDisplayName);
  await expect(page.locator('.workspace-fleet')).toContainText('Vertical Codex Agent');
  await expect(page.locator('.desktop-runtime')).toHaveCount(0);
  expect(await page.evaluate(() => '__TAURI_INTERNALS__' in window)).toBe(false);
}

async function enterPublicLobby(page: Page, catalogId: string): Promise<string> {
  await page.goto(`/lobby/${catalogId}`);
  await expect
    .poll(() => new URL(page.url()).pathname, { timeout: 45_000 })
    .toContain(`/lobby/${catalogId}/instance/`);
  const encodedRoomId = new URL(page.url()).pathname.split('/instance/')[1];
  if (encodedRoomId === undefined) {
    throw new Error('公共大厅路由缺少 Matrix 房间标识。');
  }
  const roomId = decodeURIComponent(encodedRoomId);
  expect(roomId).toMatch(/^!.+:.+$/u);
  return roomId;
}

async function publishHumanMessage(page: Page, title: string, body: string): Promise<void> {
  await page.getByRole('button', { name: /Open the message composer|打开消息发送器/u }).click();
  await expect(page.locator('.message-composer__identity')).toContainText('Local Collaborator', {
    timeout: 40_000,
  });
  await page.getByLabel(/Preview title|预览标题/u).fill(title);
  await page.getByLabel(/Preview summary|预览摘要/u).fill('Cross-account cloud message closure.');
  await page.getByLabel(/Full content|完整正文/u).fill(body);
  await page.getByRole('button', { name: /Send message|发送消息/u }).click();
  await expect(page.getByText(/Message accepted|消息已接受/u)).toBeVisible({ timeout: 45_000 });
  await page
    .getByRole('button', {
      name: /Close and discard the composer|关闭并丢弃发送器/u,
    })
    .click();
}

async function verifyRemoteMessage(page: Page, title: string, body: string): Promise<void> {
  const headline = page.locator('.message-dock__headline').filter({ hasText: title });
  await expect(headline).toBeVisible({ timeout: 45_000 });
  await headline.click();
  await expect(page.locator('.content-inspector')).toContainText('Local Collaborator');
  await page.getByRole('button', { name: /Open full content|打开完整正文/u }).click();
  await expect(page.locator('.content-inspector__verified')).toContainText(body, {
    timeout: 45_000,
  });
  await page.getByRole('button', { name: /Close message details|关闭消息详情/u }).click();
}

async function createTargetedHandoffFromMessage(
  page: Page,
  title: string,
  body: string,
  targetInstanceId: string,
) {
  const headline = page.locator('.message-dock__headline').filter({ hasText: title });
  await expect(headline).toBeVisible({ timeout: 45_000 });
  await headline.click();
  await page.getByRole('button', { name: /Open full content|打开完整正文/u }).click();
  await expect(page.locator('.content-inspector__verified')).toContainText(body, {
    timeout: 45_000,
  });
  await page.getByRole('button', { name: /Give to Agent|交给 Agent/u }).click();
  await expect(
    page.getByRole('heading', { name: /Approve one-time context|批准一次性上下文/u }),
  ).toBeVisible({
    timeout: 45_000,
  });
  const target = page.locator(`input[name="handoff-target"][value="${targetInstanceId}"]`);
  await expect(target).toBeVisible({ timeout: 45_000 });
  await target.check();
  const responsePromise = page.waitForResponse(isHandoffCreationResponse);
  await page.getByRole('button', { name: /Confirm handoff|确认交付/u }).click();
  const response = await responsePromise;
  expect([200, 201]).toContain(response.status());
  return handoffResponseSchema.parse(await response.json());
}

function isHandoffCreationResponse(response: Response): boolean {
  const url = new URL(response.url());
  return (
    url.origin === apiOrigin &&
    url.pathname === '/handoffs' &&
    response.request().method() === 'POST'
  );
}

function readInput(environment: NodeJS.ProcessEnv): ProductClosureInput | null {
  const candidate = {
    agentId: environment.AGENT_ROOM_PRODUCT_CLOSURE_AGENT_ID,
    catalogId: environment.AGENT_ROOM_PRODUCT_CLOSURE_CATALOG_ID,
    collaboratorPassword: environment.AGENT_ROOM_PRODUCT_CLOSURE_COLLABORATOR_PASSWORD,
    collaboratorUsername: environment.AGENT_ROOM_PRODUCT_CLOSURE_COLLABORATOR_USERNAME,
    ownerPassword: environment.AGENT_ROOM_PRODUCT_CLOSURE_OWNER_PASSWORD,
    ownerUsername: environment.AGENT_ROOM_PRODUCT_CLOSURE_OWNER_USERNAME,
    resultPath: environment.AGENT_ROOM_PRODUCT_CLOSURE_RESULT,
    targetInstanceId: environment.AGENT_ROOM_PRODUCT_CLOSURE_TARGET_INSTANCE_ID,
  };
  if (Object.values(candidate).some((value) => value === undefined)) {
    return null;
  }
  return z
    .object({
      agentId: uuidV7Schema,
      catalogId: uuidV7Schema,
      collaboratorPassword: z.string().min(1),
      collaboratorUsername: z.string().min(1),
      ownerPassword: z.string().min(1),
      ownerUsername: z.string().min(1),
      resultPath: z.string().min(1),
      targetInstanceId: uuidV7Schema,
    })
    .parse(candidate);
}

function missingInput(): ProductClosureInput {
  throw new Error('产品闭环纵向验收输入缺失。');
}
