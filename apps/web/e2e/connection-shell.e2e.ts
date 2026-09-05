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

test('桌面连接舱呈现真实 401 登录态与五段生命周期', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 1_000, width: 1_440 });
  await page.goto('/connect');

  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Operator sign-in required|需要登录操作者账户/u,
  );
  await expect(page.getByRole('button', { name: /Sign in|登录 Agent Room/u })).toBeEnabled();
  await expect(page.locator('.connection-step')).toHaveCount(5);
  const columns = await page.locator('.connection-shell').evaluate((element) =>
    getComputedStyle(element)
      .gridTemplateColumns.split(' ')
      .map((value) => Number.parseFloat(value)),
  );
  expect(columns).toHaveLength(2);
  expect((columns[0] ?? 0) / ((columns[0] ?? 0) + (columns[1] ?? 0))).toBeCloseTo(0.3, 2);
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
  const details = page.getByRole('button', { name: /Connection details|连接详情/iu });
  await expect(details).toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('.connection-steps')).toBeHidden();
  await details.focus();
  await page.keyboard.press('Enter');
  await expect(details).toHaveAttribute('aria-expanded', 'true');
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
    /cannot be resolved|无法解析/u,
  );
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
