import { existsSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { z } from 'zod';
import { collectUnhandledFailures, connectLiveSession } from './support/live-session';

// import.meta.url 位于 apps/web/e2e-live，验收产物存于仓库根目录。
const work = new URL('../../../artifacts/private-chat/', import.meta.url);
const scenarioSchema = z.object({
  catalogId: z.string(),
  targetName: z.string(),
  targetMatrixUserId: z.string(),
  publicRoomId: z.string(),
  matrixBaseUrl: z.url(),
  firstText: z.string(),
  firstReply: z.string(),
  secondText: z.string(),
  secondReply: z.string(),
});
function put(name: string, value: object): void {
  const pending = new URL(`${name}.pending`, work);
  writeFileSync(pending, JSON.stringify(value));
  renameSync(pending, new URL(name, work));
}
async function wait(name: string): Promise<unknown> {
  const file = new URL(name, work);
  await expect.poll(() => existsSync(file), { timeout: 90_000 }).toBe(true);
  const value: unknown = JSON.parse(readFileSync(file, 'utf8'));
  return value;
}

test('原生 SAS 错码拒绝、双向加密私聊与设备重启恢复', async ({ page }) => {
  const raw: unknown = JSON.parse(readFileSync(new URL('input.json', work), 'utf8'));
  const scenario = scenarioSchema.parse(raw);
  const password = process.env.AGENT_ROOM_PRIVATE_CHAT_PASSWORD;
  if (!password) throw new Error('Run this test through tools/private_chat.py.');
  const failures = collectUnhandledFailures(page);
  const uploads: string[] = [];
  page.on('request', (request) => {
    if (request.method() === 'POST' && new URL(request.url()).pathname.includes('/contents'))
      uploads.push(request.url());
  });
  const userId = await connectLiveSession(page, {
    username: 'developer',
    password,
    expectedDisplayName: 'Local Developer',
  });
  await page.goto('/settings/security');
  await expect(page.locator('.security-account-line')).toContainText(userId, { timeout: 40_000 });
  await page.getByRole('button', { name: 'Establish encrypted identity', exact: true }).click();
  await expect(
    page.getByRole('button', { name: 'Establish encrypted identity', exact: true }),
  ).toHaveCount(0);
  const storedSession: unknown = await page.evaluate((): unknown =>
    JSON.parse(sessionStorage.getItem('agent-room.matrix-session.v1') ?? 'null'),
  );
  const { deviceId, accessToken } = z
    .object({ deviceId: z.string(), accessToken: z.string() })
    .parse(storedSession);
  await page.goto(`/lobby/${scenario.catalogId}`);
  await expect(page).toHaveURL(/\/instance\//u);
  await expect(page.locator('.lobby-scene__canvas')).toBeVisible();
  async function openPrivate(): Promise<void> {
    await page.getByRole('button', { name: 'Find a character', exact: true }).click();
    const members = page.getByRole('dialog', { name: 'Agents in this room', exact: true });
    await members.getByRole('searchbox', { name: 'Search agents' }).fill(scenario.targetName);
    await members.getByRole('button').filter({ hasText: scenario.targetName }).click();
    const inspector = page
      .locator('.agent-inspector')
      .filter({ has: page.getByRole('heading', { name: scenario.targetName, exact: true }) });
    await inspector.getByRole('button', { name: 'Message Agent', exact: true }).click();
  }
  await openPrivate();
  const direct = page.locator('.direct-conversation');
  const input = direct.getByRole('textbox', { name: 'Message', exact: true });
  await expect(input).toBeEnabled();
  const inputId = await input.getAttribute('id');
  if (!inputId?.startsWith('chat-!')) throw new Error('Private room ID is missing.');
  const roomId = inputId.slice(5);
  const before = uploads.length;
  await input.fill(scenario.firstText);
  await direct.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(direct.getByRole('alert')).toContainText('Verify this conversation’s participant');
  await expect(input).toHaveValue(scenario.firstText);
  expect(uploads).toHaveLength(before);
  await expect(direct.getByRole('log')).not.toContainText(scenario.firstText);
  put('peer.json', { roomId, userId, deviceId });
  const incoming = page.getByRole('dialog', { name: 'Verify a room participant', exact: true });
  const dialog = page.getByRole('dialog', { name: 'Verify a Matrix device', exact: true });
  for (const round of ['mismatch', 'match']) {
    await expect(incoming).toContainText(scenario.targetMatrixUserId, { timeout: 60_000 });
    await incoming.getByRole('button', { name: 'Review codes' }).click();
    const decimalLine = dialog.getByText(/^Decimal check:/u);
    await expect(decimalLine).toBeVisible({ timeout: 60_000 });
    const decimals = (await decimalLine.innerText()).match(/\d+/gu)?.map(Number);
    expect(decimals).toHaveLength(3);
    put(`${round}-browser.json`, { decimals });
    if (round === 'mismatch') {
      await expect(dialog).toContainText('Verification cancelled', { timeout: 60_000 });
      await dialog.getByRole('button', { name: 'Close', exact: true }).click();
      put('mismatch-closed.json', { cancelled: true });
    } else {
      const native = z
        .object({ decimals: z.array(z.number()).length(3) })
        .parse(await wait('match-native.json'));
      expect(decimals).toEqual(native.decimals);
      await page.setViewportSize({ width: 390, height: 844 });
      await expect(
        dialog.getByRole('button', { name: 'They match', exact: true }),
      ).toBeInViewport();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        390,
      );
      await page.screenshot({ path: fileURLToPath(new URL('sas-mobile.png', work)) });
      await dialog.getByRole('button', { name: 'They match', exact: true }).click();
      await expect(dialog).toContainText('Device verified', { timeout: 60_000 });
      await dialog.getByRole('button', { name: 'Close', exact: true }).click();
      await page.setViewportSize({ width: 1440, height: 900 });
    }
  }
  await wait('verified.json');
  for (const phase of ['first', 'second'] as const) {
    if (phase === 'first') {
      await direct.getByRole('button', { name: 'Retry this message' }).click();
    } else {
      await input.fill(scenario.secondText);
      await direct.getByRole('button', { name: 'Send', exact: true }).click();
    }
    const text = scenario[`${phase}Text`];
    const reply = scenario[`${phase}Reply`];
    await expect(direct.getByRole('log')).toContainText(text);
    const messageId = await direct
      .locator('[data-conversation-message-id]')
      .filter({ hasText: text })
      .getAttribute('data-conversation-message-id');
    expect(messageId).toBeTruthy();
    put(`${phase}-sent.json`, { roomId, messageId });
    await wait(`${phase}-replied.json`);
    await expect(direct.getByRole('log')).toContainText(reply, { timeout: 60_000 });
    await expect(
      direct.getByRole('log').locator('blockquote').filter({ hasText: text }),
    ).toBeVisible();
    if (phase === 'first') {
      put('restart-request.json', { ready: true });
      await wait('restarted.json');
      await page.reload();
      await expect(page.locator('.lobby-scene__canvas')).toBeVisible();
      if ((await input.count()) === 0) await openPrivate();
      await expect(input).toBeEnabled();
      await expect(direct.getByRole('log')).toContainText(scenario.firstReply, { timeout: 40_000 });
    }
  }
  await page.screenshot({ path: fileURLToPath(new URL('private-roundtrip.png', work)) });
  await page.getByRole('button', { name: /^Room chat/u }).click();
  const publicLog = page.locator('.workspace-room-content').getByRole('log');
  for (const text of [
    scenario.firstText,
    scenario.firstReply,
    scenario.secondText,
    scenario.secondReply,
  ])
    await expect(publicLog).not.toContainText(text);
  const publicEncryption = `/_matrix/client/v3/rooms/${encodeURIComponent(scenario.publicRoomId)}/state/m.room.encryption/`;
  expect(
    failures.filter(
      (value) => !(value.startsWith('HTTP 404 ') && value.includes(publicEncryption)),
    ),
  ).toEqual([]);
  // 关闭页面后撤销隔离账号的当前 Matrix 设备，避免预期 401 混入 UI 故障统计。
  const request = page.request;
  await page.close();
  const revoked = await request.post(`${scenario.matrixBaseUrl}/_matrix/client/v3/logout`, {
    headers: { Authorization: `Bearer ${accessToken}` },
    data: {},
  });
  expect(revoked.status()).toBe(200);
  put('revoked.json', { deviceId });
  await wait('revoked-send-blocked.json');
  put('done.json', {
    sasMismatchRejected: true,
    sasMatched: true,
    unverifiedSendBlocked: true,
    humanToAgent: true,
    agentToHuman: true,
    replyRelation: true,
    privateAbsentFromPublic: true,
    browserReloadRestored: true,
    pixiRenderer: true,
    mobileVerification: true,
    revokedPeerSendBlocked: true,
  });
});
