import { existsSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { z } from 'zod';
import {
  collectUnhandledFailures,
  connectLiveSession,
  readMatrixSession,
} from './support/live-session';
import { storedMatrixSessionSchema } from '../src/features/session/domain/matrix-session-vault';

// import.meta.url 位于 apps/web/e2e-live，验收产物存于仓库根目录。
const work = new URL('../../../artifacts/private-chat/', import.meta.url);
const scenarioSchema = z.object({
  catalogId: z.string(),
  targetName: z.string(),
  targetMatrixUserId: z.string(),
  publicRoomId: z.string(),
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

test('无需核对即可双向加密私聊，重启后恢复', async ({ page }) => {
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
  // 登录后自动签好这台设备（ADR 0011），不用输恢复密钥、不用核对。
  // Matrix ID 在最下面的“账户详情”里（界面翻新 4b 起默认收起）。
  const accountDetails = page.locator('.security-account-details');
  const signed = page.getByText('This device is ready', { exact: true });
  await expect
    .poll(
      async () => {
        await page.goto('/settings/security');
        await accountDetails.locator('summary').click({ timeout: 40_000 });
        await expect(accountDetails).toContainText(userId, { timeout: 40_000 });
        return await signed.count();
      },
      { timeout: 60_000 },
    )
    .toBe(1);
  // 会话存在 IndexedDB 里，sessionStorage 里的旧键登录后就清掉了。
  const { deviceId } = storedMatrixSessionSchema.parse(await readMatrixSession(page));
  await page.goto(`/lobby/${scenario.catalogId}`);
  await expect(page).toHaveURL(/\/instance\//u);
  const scene = page.getByRole('listbox', { name: 'Interactive Agent room scene', exact: true });
  // 画布对读屏隐藏（aria-hidden），只能在场景里按类名找；它出现说明用的是 Pixi 渲染。
  const canvas = scene.locator('.lobby-scene__canvas');
  await expect(canvas).toBeVisible();
  async function openPrivate(): Promise<void> {
    await page.getByRole('button', { name: 'Find someone', exact: true }).click();
    const members = page.getByRole('dialog', { name: 'Agents in this room', exact: true });
    await members.getByRole('searchbox', { name: 'Search agents' }).fill(scenario.targetName);
    await members.getByRole('button').filter({ hasText: scenario.targetName }).click();
    // Agent 详情最上面就是私聊按钮；它不在线时叫“留言”，打开的是同一段私聊。
    await page
      .getByRole('complementary', { name: scenario.targetName, exact: true })
      .getByRole('button', { name: /^(?:Message|Leave a message)$/u })
      .click();
  }
  await openPrivate();
  const direct = page.getByRole('region', { name: 'Direct messages', exact: true });
  const input = direct.getByRole('textbox', { name: 'Message', exact: true });
  await expect(input).toBeEnabled();
  const inputId = await input.getAttribute('id');
  if (!inputId?.startsWith('chat-!')) throw new Error('Private room ID is missing.');
  const roomId = inputId.slice(5);
  put('peer.json', { roomId, userId, deviceId });
  async function roundtrip(phase: 'first' | 'second'): Promise<void> {
    const text = scenario[`${phase}Text`];
    const reply = scenario[`${phase}Reply`];
    const before = uploads.length;
    await input.fill(text);
    await direct.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(direct.getByRole('log')).toContainText(text);
    // 加密房间只上传密文正文，发送前不再要求核对任何参与者。
    expect(uploads.length).toBeGreaterThan(before);
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
  }
  await roundtrip('first');
  put('restart-request.json', { ready: true });
  await wait('restarted.json');
  await page.reload();
  await expect(canvas).toBeVisible();
  if ((await input.count()) === 0) await openPrivate();
  await expect(input).toBeEnabled();
  await expect(direct.getByRole('log')).toContainText(scenario.firstReply, { timeout: 40_000 });
  await roundtrip('second');
  await page.screenshot({ path: fileURLToPath(new URL('private-roundtrip.png', work)) });
  // 工具栏的“对话”回到房间里的聊天；有未读时名字后面带着条数。
  await page.getByRole('button', { name: /^Chat/u }).click();
  const publicLog = page
    .getByRole('region', { name: 'Conversation', exact: true })
    .getByRole('log');
  await expect(publicLog).toBeVisible();
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
  put('done.json', {
    identityEstablishedAutomatically: true,
    chatWithoutVerification: true,
    humanToAgent: true,
    agentToHuman: true,
    replyRelation: true,
    privateAbsentFromPublic: true,
    browserReloadRestored: true,
    pixiRenderer: true,
  });
});
