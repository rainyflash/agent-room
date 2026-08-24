import { expect, test, type Page } from '@playwright/test';

const apiOrigin = 'https://api.agent-room.localhost:18443';
const matrixOrigin = 'https://matrix.agent-room.localhost:18443';
const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;

test('OIDC 与 Matrix SSO 建立同一主体并能刷新恢复', async ({ page }) => {
  test.skip(username === undefined || password === undefined, '缺少隔离验收账户。');
  const failures = collectUnhandledFailures(page);

  await page.goto('/connect');
  await page.getByRole('button', { name: /Sign in to Agent Room|登录 Agent Room/u }).click();

  await expect(page).toHaveURL(/\/realms\/agent-room\/protocol\/openid-connect\/auth/u);
  await page.locator('input[name="username"]').fill(username ?? '');
  await page.locator('input[name="password"]').fill(password ?? '');
  await page.locator('input[type="submit"], button[type="submit"]').click();

  await expect(page).toHaveURL(/\/connect(?:\?|$)/u);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Matrix device connection required|需要连接 Matrix 设备/u,
  );
  await expect(page.locator('.identity-summary')).toContainText('Local Developer');

  await page.getByRole('button', { name: /Connect Matrix device|连接 Matrix 设备/u }).click();
  await continueThroughMatrixConsentWhenRequired(page);
  await expect(page).toHaveURL(/\/connect(?:\?|$)/u, { timeout: 40_000 });
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Connection established|连接已建立/u,
    { timeout: 40_000 },
  );

  const firstIdentity = await page.locator('.identity-summary dd').first().textContent();
  expect(firstIdentity).toMatch(/^@user-[a-f0-9]{32}:matrix\.agent-room\.localhost$/u);
  await expect(page).not.toHaveURL(/loginToken=/u);
  expect(await hasUsableMatrixSession(page)).toBe(true);

  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Connection established|连接已建立/u,
    { timeout: 40_000 },
  );
  await expect(page.locator('.identity-summary dd').first()).toHaveText(firstIdentity ?? '');
  expect(failures).toEqual([]);
});

async function continueThroughMatrixConsentWhenRequired(page: Page): Promise<void> {
  const continueLink = page.getByRole('link', { name: /^Continue$/u });
  const readyHeading = page.getByRole('heading', {
    level: 1,
    name: /Connection established|连接已建立/u,
  });
  await expect
    .poll(
      async () => {
        if (await continueLink.isVisible()) {
          return 'consent';
        }
        if (await readyHeading.isVisible()) {
          return 'ready';
        }
        return 'pending';
      },
      { timeout: 40_000 },
    )
    .not.toBe('pending');

  if (await continueLink.isVisible()) {
    await continueLink.click();
  }
}

function collectUnhandledFailures(page: Page): string[] {
  const failures: string[] = [];
  page.on('pageerror', (error) => {
    failures.push(error.message);
  });
  page.on('console', (message) => {
    const browserNetworkDiagnostic = message.text().startsWith('Failed to load resource:');
    if (message.type() === 'error' && !browserNetworkDiagnostic) {
      failures.push(message.text());
    }
  });
  page.on('response', (response) => {
    if (response.status() >= 400 && !isExpectedHttpBoundary(response.status(), response.url())) {
      const url = new URL(response.url());
      failures.push(`HTTP ${String(response.status())} ${url.origin}${url.pathname}`);
    }
  });
  return failures;
}

function isExpectedHttpBoundary(status: number, rawUrl: string): boolean {
  const url = new URL(rawUrl);
  return (
    (status === 401 && url.origin === apiOrigin && url.pathname === '/auth/session') ||
    (status === 404 &&
      url.origin === matrixOrigin &&
      url.pathname === '/_matrix/client/unstable/org.matrix.msc4143/rtc/transports')
  );
}

async function hasUsableMatrixSession(page: Page): Promise<boolean> {
  return await page.evaluate(() => {
    const serialized = window.sessionStorage.getItem('agent-room.matrix-session.v1');
    if (serialized === null) {
      return false;
    }
    try {
      const value: unknown = JSON.parse(serialized);
      if (typeof value !== 'object' || value === null) {
        return false;
      }
      const session = value as Record<string, unknown>;
      return (
        typeof session.accessToken === 'string' &&
        session.accessToken.length > 0 &&
        typeof session.deviceId === 'string' &&
        session.deviceId.length > 0 &&
        typeof session.userId === 'string' &&
        session.userId.length > 0
      );
    } catch {
      return false;
    }
  });
}
