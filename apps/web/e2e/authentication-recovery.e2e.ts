import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`验证失败保留旧会话并显示继续入口（${String(width)}px）`, async ({
    page,
    baseURL,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    const started: string[] = [];
    if (baseURL === undefined) throw new Error('Missing browser base URL');
    const headers = {
      'access-control-allow-credentials': 'true',
      'access-control-allow-origin': new URL(baseURL).origin,
      'content-type': 'application/json',
    };
    await page.route('https://api.agent-room.localhost:18443/**', async (route) => {
      const url = new URL(route.request().url());
      if (url.pathname === '/auth/session') {
        await route.fulfill({
          headers,
          json: {
            authenticatedAtUnixMs: Date.now() - 3600_000,
            expiresAtUnixMs: Date.now() + 3600_000,
            displayName: 'Returning member',
            locale: 'en',
            matrixUserId: '@human:matrix.test',
            principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
            recentlyAuthenticated: false,
          },
        });
      } else if (url.pathname === '/health/ready') {
        await route.fulfill({
          headers,
          json: {
            checkedAtUnixMs: Date.now(),
            correlationId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
            dependencies: [],
            service: 'agent-room-control-plane',
            status: 'ready',
            version: '0.1.0',
          },
        });
      } else if (url.pathname === '/auth/oidc/start') {
        started.push(url.searchParams.get('returnTo') ?? '');
        await route.fulfill({
          contentType: 'text/html',
          body: '<h1>New identity verification request</h1>',
        });
      } else {
        await route.fulfill({ headers, status: 204 });
      }
    });
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/connect?authentication=expired');
    await expect(page).toHaveTitle('Agent Room');
    await expect(
      page.getByRole('heading', {
        name: /Identity verification did not finish|这次身份验证未完成/u,
      }),
    ).toBeVisible();
    await expect(
      page.getByText(/existing account session is still signed in|原有账户仍然保持登录/u),
    ).toBeVisible();
    await expect(page).toHaveURL(/\/connect\?authentication=expired$/u);
    await page.reload();
    await expect(
      page.getByText(/permission change has not been approved|这次权限变更尚未获批/u),
    ).toBeVisible();
    await expectNoHorizontalOverflow(page);
    const retry = page.getByRole('button', { name: /Continue verification|继续验证/u });
    await expect(retry).toBeInViewport();
    await page.screenshot({
      path: testInfo.outputPath(`authentication-recovery-${String(width)}.png`),
      fullPage: true,
    });
    expect(failures).toEqual([]);
    await retry.click();
    await expect(
      page.getByRole('heading', { name: 'New identity verification request' }),
    ).toBeVisible();
    expect(started).toEqual(['/rooms']);
  });
}
