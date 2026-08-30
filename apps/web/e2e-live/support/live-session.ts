import { expect, type Locator, type Page } from '@playwright/test';

export const apiOrigin = 'https://api.agent-room.localhost:18443';
export const matrixOrigin = 'https://matrix.agent-room.localhost:18443';

export type LiveSessionCredentials = Readonly<{
  expectedDisplayName: string;
  password: string;
  username: string;
}>;

export async function connectLiveSession(
  page: Page,
  credentials: LiveSessionCredentials,
): Promise<string> {
  const login = page.getByRole('button', {
    name: /Sign in to Agent Room|登录 Agent Room/u,
  });
  await openConnectionPage(page, login);
  await login.click();

  await expect(page).toHaveURL(/\/realms\/agent-room\/protocol\/openid-connect\/auth/u);
  const usernameInput = page.locator('input[name="username"]');
  await waitForVisibleSurface(page, usernameInput, 'OIDC 登录表单');
  await usernameInput.fill(credentials.username);
  await page.locator('input[name="password"]').fill(credentials.password);
  await page.locator('input[type="submit"], button[type="submit"]').click();

  await expect(page).toHaveURL(/\/connect(?:\?|$)/u);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Matrix device connection required|需要连接 Matrix 设备/u,
  );
  await expect(page.locator('.identity-summary')).toContainText(credentials.expectedDisplayName);

  await page.getByRole('button', { name: /Connect Matrix device|连接 Matrix 设备/u }).click();
  await continueThroughMatrixConsentWhenRequired(page);
  await expect(page).toHaveURL(/\/connect(?:\?|$)/u, { timeout: 40_000 });
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Connection established|连接已建立/u,
    { timeout: 40_000 },
  );

  const matrixUserId = await page.locator('.identity-summary dd').first().textContent();
  expect(matrixUserId).toMatch(/^@user-[a-f0-9]{32}:matrix\.agent-room\.localhost$/u);
  await expect(page).not.toHaveURL(/loginToken=/u);
  expect(await hasUsableMatrixSession(page)).toBe(true);
  return matrixUserId ?? '';
}

async function openConnectionPage(page: Page, login: Locator): Promise<void> {
  const runtimeErrors: string[] = [];
  const captureRuntimeError = (error: Error): void => {
    runtimeErrors.push(error.message);
  };
  page.on('pageerror', captureRuntimeError);
  try {
    await page.goto('/connect');
    await waitForVisibleSurface(page, login, '连接页', runtimeErrors);
  } finally {
    page.off('pageerror', captureRuntimeError);
  }
}

async function waitForVisibleSurface(
  page: Page,
  locator: Locator,
  surfaceName: string,
  runtimeErrors: readonly string[] = [],
): Promise<void> {
  try {
    await locator.waitFor({ state: 'visible', timeout: 20_000 });
  } catch (error: unknown) {
    const heading = await page
      .getByRole('heading', { level: 1 })
      .first()
      .textContent({ timeout: 1_000 })
      .catch(() => null);
    const body = await page
      .locator('body')
      .innerText({ timeout: 1_000 })
      .catch(() => '');
    const cause = error instanceof Error ? error.message : String(error);
    throw new Error(
      [
        `${surfaceName}未进入可交互状态。`,
        `URL: ${page.url()}`,
        `标题: ${heading?.trim() ?? '无'}`,
        `页面: ${body.trim().slice(0, 500) || '空白'}`,
        `运行时错误: ${runtimeErrors.join(' | ') || '无'}`,
        `等待失败: ${cause}`,
      ].join('\n'),
      { cause: error },
    );
  }
}

export function collectUnhandledFailures(page: Page): string[] {
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

export async function readMatrixSession(page: Page): Promise<unknown> {
  return await page.evaluate(() => {
    const serialized = window.sessionStorage.getItem('agent-room.matrix-session.v1');
    if (serialized === null) {
      return null;
    }
    try {
      return JSON.parse(serialized) as unknown;
    } catch {
      return null;
    }
  });
}

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

function isExpectedHttpBoundary(status: number, rawUrl: string): boolean {
  const url = new URL(rawUrl);
  const missingInitialPreferences =
    url.pathname.startsWith('/_matrix/client/v3/user/') &&
    url.pathname.endsWith('/account_data/io.github.rainyflash.agentroom.preferences.v1');
  return (
    (status === 401 && url.origin === apiOrigin && url.pathname === '/auth/session') ||
    (status === 404 &&
      url.origin === matrixOrigin &&
      (url.pathname === '/_matrix/client/unstable/org.matrix.msc4143/rtc/transports' ||
        url.pathname === '/_matrix/client/v3/room_keys/version' ||
        missingInitialPreferences))
  );
}

async function hasUsableMatrixSession(page: Page): Promise<boolean> {
  const session = await readMatrixSession(page);
  if (typeof session !== 'object' || session === null) {
    return false;
  }
  const values = session as Record<string, unknown>;
  return (
    typeof values.accessToken === 'string' &&
    values.accessToken.length > 0 &&
    typeof values.deviceId === 'string' &&
    values.deviceId.length > 0 &&
    typeof values.userId === 'string' &&
    values.userId.length > 0
  );
}
