import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const apiOrigin = 'https://api.agent-room.localhost:18443';
const MAC_USER_AGENT =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36';
const configuredDownloadUrl = normalizedDownloadUrl(
  process.env.VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL,
);
const configuredMacosDownloadUrl = normalizedDownloadUrl(
  process.env.VITE_AGENT_ROOM_MACOS_DOWNLOAD_URL,
);
const downloadState = configuredDownloadUrl === null ? 'pending' : 'published';
const registrationOpen = process.env.VITE_AGENT_ROOM_IDENTITY_REGISTRATION_MODE === 'open-email';
const wcagTags = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] as const;

test('公开首页呈现真实 Alpha 入口并进入房间入口', async ({ baseURL, page }, testInfo) => {
  const failures = collectPageFailures(page);
  await mockApi(page, baseURL, null);
  await page.setViewportSize({ height: 960, width: 1_440 });
  await page.goto('/');

  await expect(page).toHaveTitle('Agent Room');
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Bring any AI agent into your room|一句话，把任何/u,
  );
  await expect(page.getByRole('button', { name: /Log in|登录/u })).toBeVisible();
  const registration = registrationOpen
    ? page.getByRole('button', { name: /Create account|注册账户/u })
    : page.getByRole('button', { name: /Registration coming soon|注册即将开放/u });
  await expect(registration).toBeVisible();
  if (!registrationOpen) {
    await expect(registration).toBeDisabled();
  }

  // 下载按钮给的必须是访客这台机器的安装包，所以期望值要按浏览器自报的系统来定。
  await expectDownloadForVisitorSystem(page);

  await expectNoHorizontalOverflow(page);
  const accessibility = await new AxeBuilder({ page }).withTags([...wcagTags]).analyze();
  expect(accessibility.violations).toEqual([]);
  expect(unexpected(failures)).toEqual([]);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath(`landing-${downloadState}-desktop.png`),
  });

  await page.getByRole('link', { name: /Enter the room|进入房间/u }).click();
  await expect(page).toHaveURL(/\/connect$/u);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Welcome to Agent Room|欢迎来到 Agent Room/u,
  );
  await expect(page.locator('.connection-step')).toHaveCount(5);
});

test('已登录的访客在首页顶栏直接进入房间', async ({ baseURL, page }) => {
  const failures = collectPageFailures(page);
  await mockApi(page, baseURL, {
    authenticatedAtUnixMs: Date.now() - 60_000,
    displayName: 'Returning member',
    expiresAtUnixMs: Date.now() + 3_600_000,
    locale: 'en',
    matrixUserId: '@human:matrix.test',
    principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
    recentlyAuthenticated: false,
  });
  await page.goto('/');

  const enter = page
    .locator('.landing__account-actions')
    .getByRole('link', { name: /Enter the room|进入房间/u });
  await expect(enter).toHaveAttribute('href', '/rooms');
  await expect(page.getByRole('button', { name: /^(Log in|登录)$/u })).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  expect(unexpected(failures)).toEqual([]);
});

test('注册入口只发送 Agent Room 注册意图', async ({ page }) => {
  await page.route(`${apiOrigin}/auth/oidc/start?**`, async (route) => {
    await route.fulfill({
      body: '<!doctype html><title>Registration boundary</title>',
      contentType: 'text/html',
      status: 200,
    });
  });
  await page.goto('/');

  if (!registrationOpen) {
    await expect(
      page.getByRole('button', { name: /Registration coming soon|注册即将开放/u }),
    ).toBeDisabled();
    return;
  }

  await page.getByRole('button', { name: /Create account|注册账户/u }).click();

  const target = new URL(page.url());
  expect(target.origin).toBe(apiOrigin);
  expect(target.pathname).toBe('/auth/oidc/start');
  expect(target.searchParams.get('returnTo')).toBe('/connect');
  expect(target.searchParams.get('intent')).toBe('register');
});

test('公开首页在 390px 视口保持主入口可用', async ({ baseURL, page }, testInfo) => {
  const failures = collectPageFailures(page);
  await mockApi(page, baseURL, null);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto('/');

  const preview = page.getByRole('link', { name: /Enter the room|进入房间/u });
  await expect(preview).toBeVisible();
  expect((await preview.boundingBox())?.height ?? 0).toBeGreaterThanOrEqual(44);
  await expectNoHorizontalOverflow(page);
  expect(unexpected(failures)).toEqual([]);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath(`landing-${downloadState}-mobile.png`),
  });
});

test('Mac 访客不会被递上 Windows 安装包', async ({ browser }) => {
  const context = await browser.newContext({ userAgent: MAC_USER_AGENT });
  const page = await context.newPage();
  try {
    await page.goto('/');

    await expect(
      page.getByRole('link', { name: /Download for Windows|下载 Windows 应用/u }),
    ).toHaveCount(0);
    await expectDownloadForVisitorSystem(page);
  } finally {
    await context.close();
  }
});

async function expectDownloadForVisitorSystem(page: Page) {
  const system = await page.evaluate(() => {
    const value = navigator.userAgent.toLowerCase();
    if (/windows|win32|win64/u.test(value)) return 'windows';
    if (/macintosh|mac os x/u.test(value)) return 'macos';
    return 'other';
  });
  const expected =
    system === 'windows'
      ? configuredDownloadUrl
      : system === 'macos'
        ? configuredMacosDownloadUrl
        : null;
  if (expected === null) {
    await expect(
      page.getByRole('button', { name: /No download for your system|暂无你系统的下载/u }),
    ).toBeDisabled();
    return;
  }
  const name =
    system === 'macos'
      ? /Download for Mac|下载 Mac 应用/u
      : /Download for Windows|下载 Windows 应用/u;
  await expect(page.getByRole('link', { name })).toHaveAttribute('href', expected);
}

/** 首页会问一下服务器是否已登录；没登录时服务器回 401，浏览器会在控制台记一条，这是预期的。 */
async function mockApi(page: Page, baseURL: string | undefined, session: object | null) {
  if (baseURL === undefined) throw new Error('Missing browser base URL');
  const headers = {
    'access-control-allow-credentials': 'true',
    'access-control-allow-origin': new URL(baseURL).origin,
    'content-type': 'application/json',
  };
  await page.route(`${apiOrigin}/**`, async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname === '/auth/session') {
      await route.fulfill(
        session === null
          ? { body: '{}', headers, status: 401 }
          : { headers, json: session, status: 200 },
      );
      return;
    }
    if (url.pathname === '/health/ready') {
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
      return;
    }
    await route.fulfill({ body: '{}', headers, status: 404 });
  });
}

function unexpected(failures: readonly string[]): readonly string[] {
  return failures.filter((text) => !text.includes('401 (Unauthorized)'));
}

function normalizedDownloadUrl(value: string | undefined): string | null {
  const normalized = value?.trim();
  return normalized === undefined || normalized.length === 0 ? null : normalized;
}
