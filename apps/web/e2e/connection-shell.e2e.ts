import { expect, test, type Page } from '@playwright/test';

const apiOrigin = 'https://api.agent-room.localhost:18443';

const readyReport = {
  checkedAtUnixMs: 1_700_000_000_000,
  correlationId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
  dependencies: [
    { latencyMs: 12, name: 'database', status: 'ready' },
    { latencyMs: 18, name: 'matrix', status: 'ready' },
  ],
  service: 'agent-room-control-plane',
  status: 'ready',
  version: '0.1.0',
};

test.beforeEach(async ({ baseURL, page }) => {
  if (!baseURL) {
    throw new Error('Playwright baseURL 未配置。');
  }
  const browserOrigin = new URL(baseURL).origin;
  await page.route(`${apiOrigin}/**`, async (route) => {
    const url = new URL(route.request().url());
    const headers = {
      'access-control-allow-credentials': 'true',
      'access-control-allow-origin': browserOrigin,
      'content-type': 'application/json',
    };
    if (url.pathname === '/auth/session') {
      await route.fulfill({ body: '{}', headers, status: 401 });
      return;
    }
    if (url.pathname === '/health/ready') {
      await route.fulfill({ body: JSON.stringify(readyReport), headers, status: 200 });
      return;
    }
    await route.fulfill({ body: '{}', headers, status: 404 });
  });
});

test('桌面宽度下连接页是一张居中的卡片，五段进度收在详情里', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 1_000, width: 1_440 });
  await page.goto('/connect');

  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Welcome to Agent Room|欢迎来到 Agent Room/u,
  );
  await expect(page.getByRole('button', { name: /Sign in|登录 Agent Room/u })).toBeEnabled();
  await expect(page.locator('.connection-step')).toHaveCount(5);
  await expect(page.locator('.connection-steps')).toBeHidden();
  const card = await page.locator('.entry-card').boundingBox();
  expect(card?.width ?? 0).toBeLessThanOrEqual(580);
  expect(Math.abs((card?.x ?? 0) + (card?.width ?? 0) / 2 - 720)).toBeLessThanOrEqual(2);
  await expectNoHorizontalOverflow(page);
  await page.keyboard.press('Tab');
  await expect(page.locator('.skip-link')).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.locator('#main-content')).toBeInViewport();
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath('connection-desktop.png'),
  });
});

test('390px 移动布局首屏可操作且连接详情可用键盘展开收起', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto('/connect');

  await expect(page.getByLabel(/Language|语言/u)).toBeVisible();
  const primaryAction = page.getByRole('button', { name: /Sign in|登录 Agent Room/u });
  await expect(primaryAction).toBeInViewport({ ratio: 1 });
  const actionBox = await primaryAction.boundingBox();
  expect(actionBox?.height ?? 0).toBeGreaterThanOrEqual(44);
  const details = page.locator('.connection-service-details > summary');
  await expect(details).toHaveText(/Connection details|连接详情/u);
  await expect(page.locator('.connection-steps')).toBeHidden();
  await details.focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('.connection-steps')).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(page.locator('.connection-steps')).toBeHidden();
  await expect(primaryAction).toBeInViewport({ ratio: 1 });

  await page.getByLabel(/Language|语言/u).selectOption('account:zh-CN');
  await expect(page.getByRole('button', { name: '登录 Agent Room' })).toBeInViewport({ ratio: 1 });
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath('connection-mobile.png'),
  });
});

test('没登录时打开房间链接，登录完回到这个房间', async ({ page }) => {
  const catalogId = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
  const started: string[] = [];
  await page.route(`${apiOrigin}/auth/oidc/start?**`, async (route) => {
    started.push(new URL(route.request().url()).searchParams.get('returnTo') ?? '');
    await route.fulfill({
      body: '<!doctype html><title>Sign-in boundary</title>',
      contentType: 'text/html',
      status: 200,
    });
  });
  await page.goto(`/lobby/${catalogId}`);
  await expect(page).toHaveURL(/\/connect\?/u);
  expect(new URL(page.url()).searchParams.get('returnTo')).toBe(`/lobby/${catalogId}`);
  await page.getByRole('button', { name: /Sign in|登录 Agent Room/u }).click();
  await expect(page).toHaveTitle('Sign-in boundary');
  expect(started).toEqual([`/lobby/${catalogId}`]);
});

test('离线刷新与无效深链都给出明确边界', async ({ context, page }) => {
  await page.goto('/connect');
  await context.setOffline(true);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /This device is offline|当前设备离线/u,
  );
  await expect(page.getByRole('button', { name: /Retry now|立即重试/u })).toBeEnabled();

  await context.setOffline(false);
  await page.goto('/lobby/bad%20catalog');
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /page does not exist|页面不存在/u,
  );
  // 出错页只给“回到房间”，地址收在详情里。
  await expect(page.getByRole('link', { name: /Back to rooms|回到房间/u })).toHaveAttribute(
    'href',
    '/rooms',
  );
  await expect(page.getByText('/lobby/bad catalog', { exact: true })).toBeHidden();
});

function collectPageFailures(page: Page): string[] {
  const failures: string[] = [];
  page.on('pageerror', (error) => {
    failures.push(error.message);
  });
  page.on('console', (message) => {
    const expectedAuthenticationMiss =
      message.type() === 'error' && message.text().includes('401 (Unauthorized)');
    if (message.type() === 'error' && !expectedAuthenticationMiss) {
      failures.push(message.text());
    }
  });
  return failures;
}

async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  const dimensions = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    offenders: [...document.querySelectorAll<HTMLElement>('body *')]
      .map((element) => ({
        className: element.className,
        left: element.getBoundingClientRect().left,
        right: element.getBoundingClientRect().right,
        tagName: element.tagName,
      }))
      .filter(
        ({ left, right }) => left < -0.5 || right > document.documentElement.clientWidth + 0.5,
      )
      .slice(0, 12),
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(
    dimensions.scrollWidth,
    `横向越界元素：${JSON.stringify(dimensions.offenders)}`,
  ).toBeLessThanOrEqual(dimensions.clientWidth);
}
