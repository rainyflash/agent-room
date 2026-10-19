import { chromium, expect, test, type BrowserContext } from '@playwright/test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';

import {
  collectUnhandledFailures,
  connectLiveSession,
  continueThroughMatrixConsentWhenRequired,
  deleteLocalCryptoStores,
  interruptFirstSigningUpload,
  readMatrixSession,
  serverDeviceIds,
  serverHasSigningIdentity,
  submitIdentityForm,
} from './support/live-session';
import { removeBuiltinsMissingOnIos172 } from './support/old-safari';
import { storedMatrixSessionSchema } from '../src/features/session/domain/matrix-session-vault';
const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;
const secondUsername = process.env.AGENT_ROOM_E2E_SECOND_USERNAME;
const secondPassword = process.env.AGENT_ROOM_E2E_SECOND_PASSWORD;
/** 登录后自动签好这台设备（ADR 0011）：真实控制面保管钥匙、真实 Synapse 上签名。 */
const thisDeviceReady = /^(?:This device is ready|这台设备已就绪)$/u;

test('OIDC 与 Matrix SSO 建立同一主体并能刷新恢复', async ({ page }) => {
  test.skip(username === undefined || password === undefined, '缺少隔离验收账户。');
  const failures = collectUnhandledFailures(page);
  // 账户第一台设备建签名身份时，第一次上传公钥断掉：之后再打开页面也得传上去，别的设备才签得上。
  const signingUploadInterrupted = await interruptFirstSigningUpload(page);

  const firstIdentity = await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });
  // 确实断在了某一页上，再往下换页面。
  await expect.poll(signingUploadInterrupted, { timeout: 40_000 }).toBe(true);

  const authenticationRequests: string[] = [];
  page.on('request', (request) => {
    if (new URL(request.url()).pathname.includes('/login/sso/redirect'))
      authenticationRequests.push(request.url());
  });
  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Find your room|找到你的房间/u,
    { timeout: 40_000 },
  );
  expect(storedMatrixSessionSchema.parse(await readMatrixSession(page)).userId).toBe(firstIdentity);
  // Matrix ID 在“设置 → 安全”最下面的“账户详情”里（界面翻新 4b 起默认收起）。
  const accountDetails = page.locator('.security-account-details');
  await page.goto('/settings/security');
  await expect(page.getByText(thisDeviceReady)).toBeVisible({ timeout: 60_000 });
  expect(await serverHasSigningIdentity(page)).toBe(true);
  await accountDetails.locator('summary').click({ timeout: 40_000 });
  await expect(accountDetails).toContainText(firstIdentity, { timeout: 40_000 });
  await page.reload();
  await accountDetails.locator('summary').click({ timeout: 40_000 });
  await expect(accountDetails).toContainText(firstIdentity, { timeout: 40_000 });
  expect(failures).toEqual([]);
  expect(authenticationRequests).toEqual([]);
});

test('真实账户关闭浏览器进程后自动恢复同一通信设备，退出后重启保持退出', async ({ baseURL }) => {
  test.skip(username === undefined || password === undefined, '缺少隔离验收账户。');
  test.setTimeout(180_000);
  const profile = await mkdtemp(join(tmpdir(), 'agent-room-live-login-'));
  let context: BrowserContext | undefined;
  const failures: string[][] = [];
  const open = async () => {
    context = await chromium.launchPersistentContext(profile, {
      ...(baseURL === undefined ? {} : { baseURL }),
      headless: true,
      ignoreHTTPSErrors: true,
      ...(process.env.CI ? {} : { channel: 'chrome' }),
    });
    const page = context.pages()[0] ?? (await context.newPage());
    failures.push(collectUnhandledFailures(page));
    return page;
  };
  try {
    const first = await open();
    const identity = await connectLiveSession(first, {
      expectedDisplayName: 'Local Developer',
      password: password ?? '',
      username: username ?? '',
    });
    // 又一台新设备：账户已有签名身份时，用服务器保管的钥匙自动签上，不用恢复密钥、不用核对。
    await first.goto('/settings/security');
    await expect(first.getByText(thisDeviceReady)).toBeVisible({ timeout: 60_000 });
    const original = storedMatrixSessionSchema.parse(await readMatrixSession(first));
    await context?.close();

    const second = await open();
    await second.goto('/connect');
    await expect(second).toHaveURL(/\/rooms$/u, { timeout: 40_000 });
    const restored = storedMatrixSessionSchema.parse(await readMatrixSession(second));
    expect(restored.deviceId).toBe(original.deviceId);
    expect(restored.userId).toBe(original.userId);
    expect(restored.userId).toBe(identity);
    expect(
      await second.evaluate(() => sessionStorage.getItem('agent-room.matrix-session.v1')),
    ).toBeNull();
    await second.screenshot({ path: join(tmpdir(), 'agent-room-live-login-restored.png') });
    await second.goto('/workspace');
    const signOut = second.getByRole('button', { name: /Sign out|退出登录/u });
    await expect(signOut).toBeVisible();
    await signOut.click();
    const signIn = second.getByRole('button', { name: /Sign in to Agent Room|登录 Agent Room/u });
    await expect(signIn).toBeVisible();
    expect(await readMatrixSession(second)).toBeNull();
    // 身份服务上的登录也一起结束了（specs/sign-out/design.md）：几分钟前刚输过密码，再点登录
    // 也不会直接回到这个账户，而是一张能填别的账户的登录页。
    await signIn.click();
    await expect(second).toHaveURL(/\/realms\/agent-room\/protocol\/openid-connect\/auth/u);
    await expect(second.locator('input[name="username"]')).toBeEditable({ timeout: 20_000 });
    await context?.close();

    const third = await open();
    await third.goto('/connect');
    await expect(
      third.getByRole('button', { name: /Sign in to Agent Room|登录 Agent Room/u }),
    ).toBeVisible();
    expect(await readMatrixSession(third)).toBeNull();
    expect(failures.flat()).toEqual([]);
  } finally {
    await context?.close();
    const pathFromTemp = relative(tmpdir(), profile);
    expect(pathFromTemp.startsWith('agent-room-live-login-') && !pathFromTemp.includes('..')).toBe(
      true,
    );
    await rm(profile, { recursive: true, force: true });
  }
});

test('旧版 iOS 浏览器（17.2）缺的内置函数都拿掉，也把这台设备的密钥传上去、签好、接着同步', async ({
  page,
}) => {
  test.skip(username === undefined || password === undefined, '缺少隔离验收账户。');
  test.setTimeout(180_000);
  // 2026-10-19 维护者的 iPhone（iOS 17.2）上，加密库同步一次就停住，这台设备的密钥一直没传上服务器，
  // 签不好，加密房间里发不出消息。
  await removeBuiltinsMissingOnIos172(page);
  const syncs: string[] = [];
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (url.pathname.endsWith('/_matrix/client/v3/sync'))
      syncs.push(url.searchParams.get('since') ?? '');
  });
  page.on('console', (message) => {
    if (message.type() === 'error' || message.type() === 'warning')
      console.log(`[页面 ${message.type()}] ${message.text().slice(0, 400)}`);
  });
  page.on('pageerror', (error) => {
    console.log(`[页面异常] ${error.stack ?? error.message}`);
  });
  const failures = collectUnhandledFailures(page);
  await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });
  syncs.length = 0;
  await page.goto('/settings/security');
  await expect(page.getByText(thisDeviceReady)).toBeVisible({ timeout: 60_000 });
  // “已就绪”是本机说的；那台 iPhone 上它也说已就绪，服务器上却没有这台设备的密钥。
  const { deviceId } = storedMatrixSessionSchema.parse(await readMatrixSession(page));
  await expect
    .poll(async () => await serverDeviceIds(page), { timeout: 40_000 })
    .toContain(deviceId);
  // 那台 iPhone 上只同步了一次就停了：重新打开以后要接着发第二次同步。
  await expect.poll(() => syncs.length, { timeout: 60_000 }).toBeGreaterThanOrEqual(2);
  expect(failures).toEqual([]);
});

test.describe('本机加密库丢了', () => {
  // 删库用的空白页由路由给出，不能让 Service Worker 换成应用。
  test.use({ serviceWorkers: 'block' });

  test('再打开时自动换一个新设备号并签好，旧设备从服务器上删掉', async ({ page }) => {
    test.skip(username === undefined || password === undefined, '缺少隔离验收账户。');
    test.setTimeout(180_000);
    const failures = collectUnhandledFailures(page);
    await connectLiveSession(page, {
      expectedDisplayName: 'Local Developer',
      password: password ?? '',
      username: username ?? '',
    });
    await page.goto('/settings/security');
    await expect(page.getByText(thisDeviceReady)).toBeVisible({ timeout: 60_000 });
    const lost = storedMatrixSessionSchema.parse(await readMatrixSession(page));
    expect(await serverDeviceIds(page)).toContain(lost.deviceId);

    // 加密库没了、会话还在：再打开时加密库会用同一个设备号新建一套密钥，Agent 不认。
    expect(await deleteLocalCryptoStores(page)).not.toEqual([]);
    await page.goto('/connect');
    await continueThroughMatrixConsentWhenRequired(page);
    await expect(page).toHaveURL(/\/rooms$/u, { timeout: 40_000 });

    const replaced = storedMatrixSessionSchema.parse(await readMatrixSession(page));
    expect(replaced.userId).toBe(lost.userId);
    expect(replaced.deviceId).not.toBe(lost.deviceId);
    await page.goto('/settings/security');
    await expect(page.getByText(thisDeviceReady)).toBeVisible({ timeout: 60_000 });
    const devices = await serverDeviceIds(page);
    expect(devices).toContain(replaced.deviceId);
    expect(devices).not.toContain(lost.deviceId);
    expect(failures).toEqual([]);
  });
});

test('设置里换个账号：退出这个账户，在身份服务的登录页登录另一个账户', async ({ page }) => {
  test.skip(
    username === undefined ||
      password === undefined ||
      secondUsername === undefined ||
      secondPassword === undefined,
    '缺少两个隔离验收账户。',
  );
  test.setTimeout(180_000);
  const failures = collectUnhandledFailures(page);
  const first = await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });

  await page.goto('/settings/account');
  await page.getByRole('button', { name: /^(?:Switch account|换个账号)$/u }).click();
  // 几分钟前刚输过密码，也不会直接回到这个账户：退出时身份服务上的登录一起结束了
  // （specs/sign-out/design.md）。
  await submitIdentityForm(page, secondUsername ?? '', secondPassword ?? '');
  // 先回到设置页，之后还要去聊天服务器登录一次（新账户第一次要点 Continue）：本机记下另一个账户的
  // 聊天会话才算换好。中途翻页时读不到就当还没好。
  const continueLink = page.getByRole('link', { name: /^Continue$/u });
  const storedUserId = async () => {
    const stored = storedMatrixSessionSchema.safeParse(
      await readMatrixSession(page).catch(() => null),
    );
    return stored.success ? stored.data.userId : null;
  };
  await expect
    .poll(
      async () => {
        if (await continueLink.isVisible().catch(() => false)) {
          await continueLink.click().catch(() => undefined);
        }
        const userId = await storedUserId();
        return userId !== null && userId !== first;
      },
      { timeout: 60_000 },
    )
    .toBe(true);
  await page.goto('/settings/account');
  await expect(page.getByText('Local Collaborator', { exact: true })).toBeVisible({
    timeout: 40_000,
  });
  expect(await storedUserId()).toMatch(/^@user-[a-f0-9]{32}:matrix\.agent-room\.localhost$/u);
  expect(failures).toEqual([]);
});
