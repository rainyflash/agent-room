import { createHash, randomBytes } from 'node:crypto';
import { isAbsolute } from 'node:path';
import { writeFile } from 'node:fs/promises';

import {
  chromium,
  expect,
  test,
  type Browser,
  type BrowserContext,
  type Page,
  type Response,
} from '@playwright/test';
import { z } from 'zod';

import {
  apiOrigin,
  collectUnhandledFailures,
  connectLiveSession,
  matrixOrigin,
  readMatrixSession,
  type LiveSessionCredentials,
} from '../e2e-live/support/live-session';

const desktopSessionCookie = '__Secure-agent-room-desktop-session';
const matrixSessionStorageKey = 'agent-room.matrix-session.v1';
const desktopOriginPattern = /^(?:http:\/\/tauri\.localhost|tauri:\/\/localhost)/u;
const appOrigin = 'https://app.agent-room.localhost:18443';

const matrixSessionSchema = z
  .object({
    accessToken: z.string().min(1),
    deviceId: z.string().min(1),
    userId: z.string().regex(/^@[^:]+:.+$/u),
  })
  .loose();

const matrixLoginResponseSchema = z
  .object({
    access_token: z.string().min(1),
    device_id: z.string().min(1),
    refresh_token: z.string().min(1).optional(),
    user_id: z.string().regex(/^@[^:]+:.+$/u),
  })
  .loose();

const desktopExchangeSchema = z
  .object({
    session: z
      .object({
        displayName: z.string().min(1),
        matrixUserId: z.string().regex(/^@[^:]+:.+$/u),
        principalId: z.uuid(),
      })
      .loose(),
    sessionSecret: z.string().min(32),
  })
  .loose();

const input = readInput(process.env);

test('真实 Windows Tauri 进程在 Bridge 离线时仍浏览云端工作区与大厅', async ({ browser, page }) => {
  test.setTimeout(240_000);
  test.skip(input === null, '缺少真实桌面闭环验收变量。');
  const scenario = input ?? missingInput();
  expect(isAbsolute(scenario.resultPath)).toBe(true);

  const credentials: LiveSessionCredentials = {
    expectedDisplayName: scenario.expectedDisplayName,
    password: scenario.password,
    username: scenario.username,
  };
  const browserFailures = collectUnhandledFailures(page);
  await connectLiveSession(page, credentials);
  const browserMatrixSession = matrixSessionSchema.parse(await readMatrixSession(page));
  await page.goto('/onboarding');
  await expect(page.locator('.onboarding-fact')).toHaveCount(4, { timeout: 45_000 });
  const expectedLobbyName = await lobbyName(page);
  const desktopSession = await issueDesktopSession(browser, page, credentials);
  const desktopMatrixSession = await issueFreshMatrixSession(browser, credentials);
  expect(desktopSession.session.matrixUserId).toBe(browserMatrixSession.userId);
  expect(desktopMatrixSession.userId).toBe(browserMatrixSession.userId);
  expect(desktopMatrixSession.deviceId).not.toBe(browserMatrixSession.deviceId);

  const desktopBrowser = await chromium.connectOverCDP(scenario.cdpUrl);
  const desktopContext = requireSingleDesktopContext(desktopBrowser.contexts());
  const desktopPage = await waitForDesktopPage(desktopContext);
  const desktopFailures = collectUnhandledFailures(desktopPage);
  await desktopContext.addCookies([
    {
      domain: 'api.agent-room.localhost',
      httpOnly: true,
      name: desktopSessionCookie,
      path: '/',
      sameSite: 'None',
      secure: true,
      value: desktopSession.sessionSecret,
    },
  ]);
  await desktopPage.evaluate(
    ({ key, session }) => {
      window.sessionStorage.setItem(key, JSON.stringify(session));
    },
    { key: matrixSessionStorageKey, session: desktopMatrixSession },
  );
  await desktopPage.reload({ waitUntil: 'domcontentloaded' });
  await navigateWithinDesktop(desktopPage, '/workspace');

  await expect(desktopPage.locator('.account-workspace')).toBeVisible({ timeout: 45_000 });
  await expect(desktopPage.getByRole('heading', { level: 1 })).toContainText(
    /Every Agent\. Every device\. One account truth\.|所有 Agent、所有设备，共用一个账户事实。/u,
  );
  await expect(desktopPage.locator('.account-workspace__intro')).toContainText(
    scenario.expectedDisplayName,
  );
  await expect(desktopPage.locator('.desktop-runtime')).toBeVisible();
  const statuses = desktopPage.locator('.workspace-status');
  await expect(statuses.nth(0)).toContainText(/Online|在线/u, { timeout: 45_000 });
  await expect(statuses.nth(1)).toContainText(/Online|在线/u, { timeout: 45_000 });

  const bridgePhase = await waitForOfflineBridge(desktopPage);
  await expect(statuses.nth(2)).toContainText(/Degraded|Offline|降级|离线/u);
  expect(await desktopPage.evaluate(() => '__TAURI_INTERNALS__' in window)).toBe(true);

  await navigateWithinDesktop(desktopPage, '/onboarding');
  await expect(desktopPage.locator('.onboarding')).toBeVisible({ timeout: 45_000 });
  const lobbyCard = defaultLobbyCard(desktopPage);
  await expect(lobbyCard).toContainText(/Default public lobby|默认公共大厅/u);
  await expect(lobbyCard).toContainText(expectedLobbyName);
  const enterLobby = desktopPage.getByRole('button', { name: /Enter lobby|进入大厅/u });
  await expect(enterLobby).toBeEnabled();
  await enterLobby.click();
  await expect(desktopPage.locator('.lobby-shell')).toBeVisible({ timeout: 45_000 });
  await expect(desktopPage).toHaveURL(/\/lobby\/[^/]+\/instance\/[^/]+/u);

  await writeFile(
    scenario.resultPath,
    `${JSON.stringify(
      {
        bridgePhase,
        controlPlaneStatus: 'online',
        desktopOrigin: new URL(desktopPage.url()).origin,
        lobbyEntered: 'true',
        lobbyName: expectedLobbyName,
        matrixStatus: 'online',
        processKind: 'tauri_webview2',
        tauriRuntimeDetected: 'true',
        workspaceVisible: 'true',
      },
      null,
      2,
    )}\n`,
    'utf8',
  );
  expect([...browserFailures, ...desktopFailures]).toEqual([]);
});

type DesktopClosureInput = Readonly<{
  cdpUrl: string;
  expectedDisplayName: string;
  password: string;
  resultPath: string;
  username: string;
}>;

async function issueDesktopSession(
  browser: Browser,
  exchangePage: Page,
  credentials: LiveSessionCredentials,
) {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, locale: 'en-US' });
  try {
    const page = await context.newPage();
    const clientState = randomUrlSafeValue();
    const pkceVerifier = randomUrlSafeValue();
    const codeChallenge = createHash('sha256').update(pkceVerifier).digest('base64url');
    const start = new URL('/auth/desktop/start', apiOrigin);
    start.searchParams.set('clientState', clientState);
    start.searchParams.set('codeChallenge', codeChallenge);
    start.searchParams.set('returnTo', '/workspace');
    start.searchParams.set('importDisplayName', 'true');
    start.searchParams.set('importLocale', 'true');
    start.searchParams.set('intent', 'sign-in');
    await page.goto(start.toString());
    await expect(page).toHaveURL(/\/realms\/agent-room\/protocol\/openid-connect\/auth/u);
    await page.locator('input[name="username"]').fill(credentials.username);
    await page.locator('input[name="password"]').fill(credentials.password);
    const callbackResponse = page.waitForResponse(isDesktopCallbackResponse);
    await page.locator('input[type="submit"], button[type="submit"]').click();
    const callback = await callbackResponse;
    const callbackUrl = callback.headers().location;
    if (!callbackUrl?.startsWith('agent-room://auth/callback')) {
      throw new Error('桌面 OIDC 回调没有返回受信任的自定义协议地址。');
    }
    const parsedCallback = new URL(callbackUrl);
    expect(parsedCallback.searchParams.get('state')).toBe(clientState);
    const authorizationCode = parsedCallback.searchParams.get('code');
    if (authorizationCode === null || authorizationCode.length === 0) {
      throw new Error('桌面 OIDC 回调缺少一次性授权码。');
    }
    const exchange = await exchangePage.evaluate(
      async ({ authorizationCode, controlPlaneOrigin, pkceVerifier }) => {
        const response = await fetch(`${controlPlaneOrigin}/auth/desktop/exchange`, {
          body: JSON.stringify({ authorizationCode, pkceVerifier }),
          headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
          method: 'POST',
        });
        return { bodyText: await response.text(), status: response.status };
      },
      { authorizationCode, controlPlaneOrigin: apiOrigin, pkceVerifier },
    );
    expect(exchange.status).toBe(200);
    const exchangeBody: unknown = JSON.parse(exchange.bodyText);
    return desktopExchangeSchema.parse(exchangeBody);
  } finally {
    await context.close();
  }
}

async function issueFreshMatrixSession(browser: Browser, credentials: LiveSessionCredentials) {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, locale: 'en-US' });
  try {
    const page = await context.newPage();
    let captureLoginToken: ((token: string) => void) | undefined;
    const loginTokenPromise = new Promise<string>((resolve) => {
      captureLoginToken = resolve;
    });
    const callbackOrigin = 'https://agent-room-desktop-acceptance.invalid';
    await page.route(`${callbackOrigin}/**`, async (route) => {
      const token = new URL(route.request().url()).searchParams.get('loginToken');
      await route.fulfill({
        body: '<!doctype html><title>Agent Room desktop acceptance</title>',
        contentType: 'text/html',
        status: token === null ? 400 : 200,
      });
      if (token !== null) captureLoginToken?.(token);
    });

    const redirect = new URL('/_matrix/client/v3/login/sso/redirect', matrixOrigin);
    redirect.searchParams.set('redirectUrl', `${callbackOrigin}/callback`);
    await page.goto(redirect.toString());
    await expect(page).toHaveURL(/\/realms\/agent-room\/protocol\/openid-connect\/auth/u);
    await page.locator('input[name="username"]').fill(credentials.username);
    await page.locator('input[name="password"]').fill(credentials.password);
    await page.locator('input[type="submit"], button[type="submit"]').click();

    const continueLink = page.getByRole('link', { name: /^Continue$/u });
    const callbackOrConsent = await Promise.race([
      loginTokenPromise.then(() => 'callback' as const),
      continueLink
        .waitFor({ state: 'visible', timeout: 40_000 })
        .then(() => 'consent' as const)
        .catch(() => 'timeout' as const),
    ]);
    if (callbackOrConsent === 'timeout') {
      throw new Error('Matrix SSO 没有进入授权确认或桌面回调。');
    }
    if (callbackOrConsent === 'consent') {
      await continueLink.click();
    }
    const loginToken = await Promise.race([
      loginTokenPromise,
      rejectAfter(20_000, 'Matrix SSO 没有返回桌面设备登录令牌。'),
    ]);
    await page.goto(`${appOrigin}/connect`);

    const exchange = await page.evaluate(
      async ({ matrixBaseUrl, token }) => {
        const response = await fetch(`${matrixBaseUrl}/_matrix/client/v3/login`, {
          body: JSON.stringify({
            initial_device_display_name: 'Agent Room Windows acceptance',
            refresh_token: true,
            token,
            type: 'm.login.token',
          }),
          headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
          method: 'POST',
        });
        return { bodyText: await response.text(), status: response.status };
      },
      { matrixBaseUrl: matrixOrigin, token: loginToken },
    );
    expect(exchange.status).toBe(200);
    const exchangeBody: unknown = JSON.parse(exchange.bodyText);
    const session = matrixLoginResponseSchema.parse(exchangeBody);
    return {
      accessToken: session.access_token,
      deviceId: session.device_id,
      ...(session.refresh_token === undefined ? {} : { refreshToken: session.refresh_token }),
      userId: session.user_id,
      version: 1 as const,
    };
  } finally {
    await context.close();
  }
}

function rejectAfter(delayMs: number, message: string): Promise<never> {
  return new Promise((_resolve, reject) => {
    setTimeout(() => {
      reject(new Error(message));
    }, delayMs);
  });
}

function isDesktopCallbackResponse(response: Response): boolean {
  const url = new URL(response.url());
  return url.origin === apiOrigin && url.pathname === '/auth/oidc/callback';
}

function randomUrlSafeValue(): string {
  return randomBytes(32).toString('base64url');
}

function requireSingleDesktopContext(contexts: readonly BrowserContext[]): BrowserContext {
  if (contexts.length !== 1) {
    throw new Error(`真实桌面进程暴露了 ${String(contexts.length)} 个浏览器上下文。`);
  }
  const context = contexts[0];
  if (context === undefined) {
    throw new Error('真实桌面进程没有可用浏览器上下文。');
  }
  return context;
}

async function waitForDesktopPage(context: BrowserContext): Promise<Page> {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    const page = context.pages().find((candidate) => desktopOriginPattern.test(candidate.url()));
    if (page !== undefined) return page;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error('没有在调试端口发现真实 Tauri WebView 页面。');
}

async function waitForOfflineBridge(page: Page): Promise<string> {
  const mark = page.locator('.desktop-runtime__mark');
  await expect
    .poll(async () => await mark.getAttribute('data-phase'), { timeout: 60_000 })
    .toBe('halted');
  return (await mark.getAttribute('data-phase')) ?? 'missing';
}

async function navigateWithinDesktop(page: Page, path: string): Promise<void> {
  await page.evaluate((nextPath) => {
    window.history.pushState(null, '', nextPath);
    window.dispatchEvent(new PopStateEvent('popstate'));
  }, path);
}

async function lobbyName(page: Page): Promise<string> {
  const card = defaultLobbyCard(page);
  const nameValue = card.locator('.onboarding-fact__value strong');
  await expect
    .poll(async () => (await nameValue.textContent())?.trim() ?? '', { timeout: 45_000 })
    .toMatch(/^(?!—$).+/u);
  const name = await nameValue.textContent();
  if (name === null || name.trim().length === 0 || name.trim() === '—') {
    throw new Error('云端引导没有返回默认公共大厅。');
  }
  return name.trim();
}

function defaultLobbyCard(page: Page) {
  const heading = page.getByRole('heading', {
    level: 2,
    name: /Default public lobby|默认公共大厅/u,
  });
  return page.locator('.onboarding-fact').filter({ has: heading });
}

function readInput(environment: NodeJS.ProcessEnv): DesktopClosureInput | null {
  const candidate = {
    cdpUrl: environment.AGENT_ROOM_DESKTOP_ACCEPTANCE_CDP_URL,
    expectedDisplayName: environment.AGENT_ROOM_DESKTOP_ACCEPTANCE_DISPLAY_NAME,
    password: environment.AGENT_ROOM_DESKTOP_ACCEPTANCE_PASSWORD,
    resultPath: environment.AGENT_ROOM_DESKTOP_ACCEPTANCE_RESULT,
    username: environment.AGENT_ROOM_DESKTOP_ACCEPTANCE_USERNAME,
  };
  if (Object.values(candidate).some((value) => value === undefined)) return null;
  return z
    .object({
      cdpUrl: z.url(),
      expectedDisplayName: z.string().min(1),
      password: z.string().min(1),
      resultPath: z.string().min(1),
      username: z.string().min(1),
    })
    .parse(candidate);
}

function missingInput(): DesktopClosureInput {
  throw new Error('真实桌面闭环验收输入缺失。');
}
