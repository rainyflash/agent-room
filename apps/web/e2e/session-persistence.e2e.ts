import { chromium, expect, test, type BrowserContext } from '@playwright/test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const fixture = '/e2e/fixtures/session-persistence.html';
const identity = 'Signed in: @persistence:matrix.test / PERSISTED_DEVICE';

test('关闭整个浏览器进程后恢复同一设备，主动退出后重启不再恢复', async ({ baseURL }) => {
  test.setTimeout(60_000);
  const profile = await mkdtemp(join(tmpdir(), 'agent-room-login-profile-'));
  let context: BrowserContext | undefined;
  const failures: string[] = [];
  const open = async () => {
    context = await chromium.launchPersistentContext(profile, {
      ...(baseURL === undefined ? {} : { baseURL }),
      headless: true,
      ...(process.env.CI ? {} : { channel: 'chrome' }),
    });
    const page = context.pages()[0] ?? (await context.newPage());
    page.on('pageerror', (error) => failures.push(error.message));
    await page.goto(fixture);
    await expect(page).toHaveTitle('Agent Room session persistence');
    return page;
  };
  try {
    const first = await open();
    await expect(first.getByRole('status')).toHaveText('Signed out');
    await first.getByRole('button', { name: 'Save test login', exact: true }).click();
    await expect(first.getByRole('status')).toHaveText('Login saved');
    await context?.close();
    const second = await open();
    await expect(second.getByRole('status')).toHaveText(identity);
    expect(await second.evaluate(() => [localStorage.length, sessionStorage.length])).toEqual([
      0, 0,
    ]);
    await second.screenshot({ path: join(tmpdir(), 'agent-room-login-restored.png') });
    await second.getByRole('button', { name: 'Sign out', exact: true }).click();
    await expect(second.getByRole('status')).toHaveText('Signed out');
    await context?.close();
    const third = await open();
    await expect(third.getByRole('status')).toHaveText('Signed out');
    expect(failures).toEqual([]);
  } finally {
    await context?.close();
    expect(
      resolve(profile).startsWith(`${resolve(tmpdir())}\\`) ||
        resolve(profile).startsWith(`${resolve(tmpdir())}/`),
    ).toBe(true);
    await rm(profile, { recursive: true, force: true });
  }
});

test('旧标签页会话迁移后新标签页可恢复，注销墓碑阻止旧令牌复活', async ({ page, context }) => {
  await page.goto(fixture);
  await page.getByRole('button', { name: 'Save legacy login', exact: true }).click();
  await page.getByRole('button', { name: 'Read saved login', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText(identity);
  expect(await page.evaluate(() => sessionStorage.length)).toBe(0);
  const other = await context.newPage();
  await other.goto(fixture);
  await expect(other.getByRole('status')).toHaveText(identity);
  await other.getByRole('button', { name: 'Sign out', exact: true }).click();
  await expect(other.getByRole('status')).toHaveText('Signed out');
  await page.getByRole('button', { name: 'Refresh saved login', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('matrix.session_superseded');
  await page.getByRole('button', { name: 'Save legacy login', exact: true }).click();
  await page.getByRole('button', { name: 'Read saved login', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('Signed out');
  await page.reload();
  await expect(page.getByRole('status')).toHaveText('Signed out');
});

test('不同服务器的持久登录隔离，设备锁在原窗口关闭后释放', async ({ page, context }) => {
  await page.goto(fixture);
  await page.getByRole('button', { name: 'Save test login', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('Login saved');
  const other = await context.newPage();
  await other.goto(`${fixture}?homeserver=https://other.matrix.test`);
  await expect(other.getByRole('status')).toHaveText('Signed out');
  await other.goto(fixture);
  await expect(other.getByRole('status')).toHaveText(identity);
  await page.getByRole('button', { name: 'Connect device', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('Device connected');
  await other.getByRole('button', { name: 'Connect device', exact: true }).click();
  await expect(other.getByRole('status')).toHaveText('matrix.session_in_use');
  await page.close();
  await other.getByRole('button', { name: 'Connect device', exact: true }).click();
  await expect(other.getByRole('status')).toHaveText('Device connected');
});
