import { expect, test, type Page } from '@playwright/test';

const principal = {
  authenticatedAtUnixMs: 1_800_000_000_000,
  displayName: '回归测试账户',
  expiresAtUnixMs: 1_900_000_000_000,
  locale: 'en',
  matrixUserId: '@regression:matrix.agent-room.localhost',
  principalId: '01990d9e-8400-7000-8000-000000000010',
  recentlyAuthenticated: true,
};

async function mockCloud(page: Page, authenticated: () => boolean): Promise<void> {
  await page.route('https://**/*', async (route) => {
    const url = new URL(route.request().url());
    const headers = {
      'access-control-allow-credentials': 'true',
      'access-control-allow-origin': new URL(page.url()).origin,
      'content-type': 'application/json',
    };
    if (url.pathname.startsWith('/_matrix/')) {
      await route.abort('connectionrefused');
      return;
    }
    if (url.pathname === '/auth/session') {
      await route.fulfill({
        body: JSON.stringify(authenticated() ? principal : {}),
        headers,
        status: authenticated() ? 200 : 401,
      });
      return;
    }
    const responses: Readonly<Record<string, unknown>> = {
      '/agents': { agents: [] },
      '/agent-instances': { instances: [] },
      '/auth/devices': { devices: [] },
      '/lobbies/public': { lobbies: [] },
      '/health/ready': {
        checkedAtUnixMs: 1_800_000_000_000,
        correlationId: '01990d9e-8400-7000-8000-000000000011',
        dependencies: [{ latencyMs: 0, name: 'matrix', status: 'unavailable' }],
        service: 'agent-room-control-plane',
        status: 'degraded',
        version: '0.1.0',
      },
    };
    const body = responses[url.pathname];
    await route.fulfill({
      body: JSON.stringify(body ?? {}),
      headers,
      status: body === undefined ? 404 : 200,
    });
  });
}

test('Matrix 连接失败时仍能从连接页进入云端工作区和房间目录', async ({ page }) => {
  await mockCloud(page, () => true);
  await page.addInitScript((matrixUserId) => {
    sessionStorage.setItem(
      'agent-room.matrix-session.v1',
      JSON.stringify({
        accessToken: 'browser-regression-fixture',
        deviceId: 'REGRESSION',
        userId: matrixUserId,
        version: 1,
      }),
    );
  }, principal.matrixUserId);
  await page.goto('/connect');
  await page.getByRole('button', { name: 'Open account workspace' }).click();
  await expect(page).toHaveURL(/\/workspace$/u);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    'Every Agent. Every device. One account truth.',
  );
  await expect(page.getByText(principal.displayName, { exact: true })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Service connections' })).toBeVisible();
  await page.getByRole('link', { name: 'Rooms', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText(
    'Choose where your Agents meet.',
  );
  await expect(page.getByText('No public room is available', { exact: true })).toBeVisible();
});

test('云端会话过期后立刻撤下工作区的旧账户信息', async ({ page }) => {
  let authenticated = true;
  await mockCloud(page, () => authenticated);
  await page.goto('/workspace');
  await expect(page.getByText(principal.displayName, { exact: true })).toBeVisible();
  await page.evaluate(() => {
    window.dispatchEvent(new Event('offline'));
  });
  authenticated = false;
  await page.evaluate(() => {
    window.dispatchEvent(new Event('online'));
  });
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Operator sign-in required');
  await expect(page.getByText(principal.displayName, { exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Open account workspace' })).toHaveCount(0);
});
