import { expect, type Locator, type Page } from '@playwright/test';
import { storedMatrixSessionSchema } from '../../src/features/session/domain/matrix-session-vault';
import { isExpectedHttpBoundary, matrixOrigin } from './http-boundary';

export { apiOrigin, matrixOrigin } from './http-boundary';

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

  await continueThroughMatrixConsentWhenRequired(page);
  await expect(page).toHaveURL(/\/rooms$/u, { timeout: 40_000 });
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Find your room|找到你的房间/u,
    { timeout: 40_000 },
  );

  const { userId: matrixUserId } = storedMatrixSessionSchema.parse(await readMatrixSession(page));
  expect(matrixUserId).toMatch(/^@user-[a-f0-9]{32}:matrix\.agent-room\.localhost$/u);
  await expect(page).not.toHaveURL(/loginToken=/u);
  await page.goto('/workspace');
  await expect(page.getByText(credentials.expectedDisplayName, { exact: true })).toBeVisible();
  await page.goto('/rooms');
  return matrixUserId;
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
    if (
      response.status() >= 400 &&
      !isExpectedHttpBoundary(response.status(), response.url(), response.request().method())
    ) {
      const url = new URL(response.url());
      failures.push(`HTTP ${String(response.status())} ${url.origin}${url.pathname}`);
    }
  });
  return failures;
}

export async function readMatrixSession(page: Page): Promise<unknown> {
  return await page.evaluate(
    async (homeserver) =>
      await new Promise<unknown>((resolve, reject) => {
        const request = indexedDB.open('agent-room.sessions.v1', 1);
        request.onerror = () => {
          reject(new Error('Could not open session database', { cause: request.error }));
        };
        request.onsuccess = () => {
          const database = request.result;
          if (!database.objectStoreNames.contains('matrix')) {
            database.close();
            resolve(null);
            return;
          }
          const transaction = database.transaction('matrix', 'readonly');
          transaction.oncomplete = () => {
            database.close();
          };
          transaction.onabort = () => {
            database.close();
            reject(new Error('Session read aborted', { cause: transaction.error }));
          };
          const read = transaction.objectStore('matrix').get(homeserver);
          read.onsuccess = () => {
            const value: unknown = read.result;
            resolve(
              typeof value === 'object' && value !== null && 'session' in value
                ? value.session
                : null,
            );
          };
        };
      }),
    matrixOrigin,
  );
}

/**
 * 把这个账户第一次上传签名公钥掐断，像登录后页面马上跳走那样：本机建好了签名身份，服务器上没有。
 * 那一页上加密库的重试（两秒后起）也一起掐断，换了一页才放行；不然重试一成功，这种情况就碰不到了，
 * 验收只是偶尔抓得到。之后再打开页面，公钥也得传上去：matrix-js-sdk 看到本机有私钥会跳过上传
 * （2026-10-02 发布 CI 抓到的），对自己的账户又只看本机记着的身份（2026-10-09 派发 CI 抓到的）。
 * 返回的函数说第一次上传掐断过没有。
 */
export async function interruptFirstSigningUpload(page: Page): Promise<() => boolean> {
  let state: 'waiting' | 'interrupting' | 'released' = 'waiting';
  // 新的一页（文档）开始了才放行，掐断那一页上的重试随它一起没了。
  page.on('domcontentloaded', () => {
    if (state === 'interrupting') state = 'released';
  });
  await page.route('**/_matrix/client/v3/keys/device_signing/upload', async (route) => {
    if (state === 'released') {
      await route.continue();
      return;
    }
    state = 'interrupting';
    await route.abort('connectionreset');
  });
  return () => state !== 'waiting';
}

/** 服务器上这个账户有没有签名身份（主签名公钥）：本机说“已就绪”不算，要服务器上真的有。 */
export async function serverHasSigningIdentity(page: Page): Promise<boolean> {
  const { accessToken, userId } = storedMatrixSessionSchema.parse(await readMatrixSession(page));
  return await page.evaluate(
    async ({ homeserver, token, user }) => {
      const response = await fetch(`${homeserver}/_matrix/client/v3/keys/query`, {
        body: JSON.stringify({ device_keys: { [user]: [] } }),
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
        method: 'POST',
      });
      const body: unknown = await response.json();
      return (
        typeof body === 'object' &&
        body !== null &&
        'master_keys' in body &&
        typeof body.master_keys === 'object' &&
        body.master_keys !== null &&
        user in body.master_keys
      );
    },
    { homeserver: matrixOrigin, token: accessToken, user: userId },
  );
}

/**
 * 删掉这个浏览器里的本机加密库，像浏览器清掉了本站存储那样，返回删掉的库名。先换到同源的空白页，
 * 应用关掉加密库的连接，删除才做得完；空白页由路由给出，调用的测试要挡住 Service Worker，免得它换成应用。
 */
export async function deleteLocalCryptoStores(page: Page): Promise<string[]> {
  await page.route('**/e2e-blank', async (route) => {
    await route.fulfill({ body: '<!doctype html><title>blank</title>', contentType: 'text/html' });
  });
  await page.goto('/e2e-blank');
  return await page.evaluate(async () => {
    const names = (await indexedDB.databases())
      .map((database) => database.name ?? '')
      .filter((name) => name.startsWith('agent-room-crypto-'));
    for (const name of names) {
      await new Promise<void>((resolve, reject) => {
        const request = indexedDB.deleteDatabase(name);
        request.onsuccess = () => {
          resolve();
        };
        request.onerror = () => {
          reject(new Error(`删不掉 ${name}`, { cause: request.error }));
        };
      });
    }
    return names;
  });
}

/** 服务器上这个账户现在有哪些传过密钥的设备。 */
export async function serverDeviceIds(page: Page): Promise<string[]> {
  const { accessToken, userId } = storedMatrixSessionSchema.parse(await readMatrixSession(page));
  return await page.evaluate(
    async ({ homeserver, token, user }) => {
      const response = await fetch(`${homeserver}/_matrix/client/v3/keys/query`, {
        body: JSON.stringify({ device_keys: { [user]: [] } }),
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
        method: 'POST',
      });
      const body = (await response.json()) as {
        device_keys?: Record<string, Record<string, unknown>>;
      };
      return Object.keys(body.device_keys?.[user] ?? {});
    },
    { homeserver: matrixOrigin, token: accessToken, user: userId },
  );
}

export async function continueThroughMatrixConsentWhenRequired(page: Page): Promise<void> {
  const continueLink = page.getByRole('link', { name: /^Continue$/u });
  await expect
    .poll(
      async () => {
        if (await continueLink.isVisible()) {
          return 'consent';
        }
        if (new URL(page.url()).pathname === '/rooms') {
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
