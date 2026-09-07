import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const userId = '@human:matrix.agent-room.localhost';
const destination = '/lobby/public?directory=open';

for (const width of [1440, 390]) {
  test(`自动接通消息、取消后停止、保留房间重试和账户退出：${String(width)}px`, async ({
    page,
    baseURL,
  }, testInfo) => {
    if (baseURL === undefined) throw new Error('缺少浏览器测试地址');
    const origin = new URL(baseURL).origin;
    const failures = collectPageFailures(page);
    let authenticated = true;
    let authorizations = 0;
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.addInitScript(
      (language) => {
        localStorage.setItem('agent-room.language', language);
      },
      width === 390 ? 'zh-CN' : 'en',
    );
    await page.route('https://**/*', async (route) => {
      const url = new URL(route.request().url());
      if (url.pathname.includes('/login/sso/redirect')) {
        authorizations += 1;
        expect(url.searchParams.get('redirectUrl')).toBe(`${origin}/connect`);
        await route.fulfill({
          contentType: 'text/html',
          body: `<!doctype html><title>Test authorization</title><a href="${origin}/connect">Cancel authorization</a>`,
        });
        return;
      }
      const headers = {
        'access-control-allow-credentials': 'true',
        'access-control-allow-origin': origin,
        'access-control-allow-headers': 'content-type',
        'access-control-allow-methods': 'GET, POST, OPTIONS',
        'content-type': 'application/json',
      };
      if (route.request().method() === 'OPTIONS') {
        await route.fulfill({ headers, status: 204 });
        return;
      }
      if (url.pathname === '/auth/logout') {
        authenticated = false;
        await route.fulfill({ headers, status: 204 });
        return;
      }
      const responses: Readonly<Record<string, unknown>> = {
        '/_matrix/client/v3/login': { flows: [{ type: 'm.login.sso' }, { type: 'm.login.token' }] },
        '/auth/session': {
          authenticatedAtUnixMs: Date.now(),
          displayName: 'Room Member',
          expiresAtUnixMs: Date.now() + 60_000,
          locale: width === 390 ? 'zh-CN' : 'en',
          matrixUserId: userId,
          principalId: '01990d9e-8400-7000-8000-000000000010',
          recentlyAuthenticated: true,
        },
        '/agents': { agents: [] },
        '/agent-instances': { instances: [] },
        '/auth/devices': { devices: [] },
        '/lobbies/public': { lobbies: [] },
        '/health/ready': {
          checkedAtUnixMs: Date.now(),
          correlationId: '01990d9e-8400-7000-8000-000000000011',
          dependencies: [{ latencyMs: 1, name: 'matrix', status: 'available' }],
          service: 'agent-room-control-plane',
          status: 'ready',
          version: '0.1.0',
        },
      };
      await route.fulfill({
        body: JSON.stringify(responses[url.pathname] ?? {}),
        headers,
        status: url.pathname === '/auth/session' && !authenticated ? 401 : 200,
      });
    });

    await page.goto(`/connect?returnTo=${encodeURIComponent(destination)}`);
    await page.getByRole('link', { name: 'Cancel authorization' }).click();
    const retry = page.getByRole('button', { name: /^(Reconnect|重新连接)$/u });
    await expect(retry).toBeVisible();
    await expect(page.getByRole('alert')).toContainText(/previous connection|上次连接/u);
    expect(authorizations).toBe(1);
    const diagnostic = page.getByText('matrix.authentication_interrupted', { exact: true });
    await expect(diagnostic).toBeHidden();
    await expect(page.getByText(userId, { exact: true })).toBeHidden();
    await expectNoHorizontalOverflow(page);
    const scan = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
      .analyze();
    expect(scan.violations).toEqual([]);
    await page.screenshot({
      fullPage: true,
      path: testInfo.outputPath(`connection-interrupted-${String(width)}.png`),
    });

    await page.locator('.connection-service-details summary').click();
    await expect(diagnostic).toBeVisible();
    await expect(page.getByText(userId, { exact: true })).toBeVisible();
    await page.reload();
    await expect(retry).toBeVisible();
    expect(authorizations).toBe(1);
    expect(
      await page.evaluate(() => sessionStorage.getItem('agent-room.matrix-return-path.v1')),
    ).toBe(destination);
    await retry.click();
    await page.getByRole('link', { name: 'Cancel authorization' }).click();
    await expect(retry).toBeVisible();
    expect(authorizations).toBe(2);
    expect(
      await page.evaluate(() => sessionStorage.getItem('agent-room.matrix-return-path.v1')),
    ).toBe(destination);

    await page.goto('/workspace');
    await expect(page.getByText('Room Member', { exact: true })).toBeVisible();
    await expectNoHorizontalOverflow(page);
    const signOut = page.getByRole('button', { name: /^(Sign out|退出登录)$/u });
    await expect(signOut).toBeVisible();
    expect(authorizations).toBe(2);
    await page.screenshot({
      fullPage: true,
      path: testInfo.outputPath(`account-signout-${String(width)}.png`),
    });
    // Capture console health before the expected signed-out /auth/session 401.
    expect(failures).toEqual([]);
    await signOut.click();
    await expect(
      page.getByRole('button', { name: /Sign in to Agent Room|登录 Agent Room/u }),
    ).toBeVisible();
    await page.reload();
    await expect(
      page.getByRole('button', { name: /Sign in to Agent Room|登录 Agent Room/u }),
    ).toBeVisible();
    expect(authorizations).toBe(2);
  });
}
