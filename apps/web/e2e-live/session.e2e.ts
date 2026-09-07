import { chromium, expect, test, type BrowserContext } from '@playwright/test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';

import {
  collectUnhandledFailures,
  connectLiveSession,
  readMatrixSession,
} from './support/live-session';
import { storedMatrixSessionSchema } from '../src/features/session/domain/matrix-session-vault';
const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;

test('OIDC 与 Matrix SSO 建立同一主体并能刷新恢复', async ({ page }) => {
  test.skip(username === undefined || password === undefined, '缺少隔离验收账户。');
  const failures = collectUnhandledFailures(page);

  const firstIdentity = await connectLiveSession(page, {
    expectedDisplayName: 'Local Developer',
    password: password ?? '',
    username: username ?? '',
  });

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
  await page.goto('/settings/security');
  await expect(page.locator('.security-account-line')).toContainText(firstIdentity, {
    timeout: 40_000,
  });
  await page.reload();
  await expect(page.locator('.security-account-line')).toContainText(firstIdentity, {
    timeout: 40_000,
  });
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
    await expect(
      second.getByRole('button', { name: /Sign in to Agent Room|登录 Agent Room/u }),
    ).toBeVisible();
    expect(await readMatrixSession(second)).toBeNull();
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
