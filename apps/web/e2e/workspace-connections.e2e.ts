import { expect, test, type Page } from '@playwright/test';

const fixturePath = '/e2e/fixtures/account-workspace.html';

test('账号工作区明确展示四层独立连接事实和诊断', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 1_000, width: 1_440 });
  await page.goto(fixturePath);

  const connections = page.getByRole('region', { name: 'Service connections' });
  await expect(connections.getByText('Control plane')).toBeVisible();
  await expect(connections.getByText('Matrix sync')).toBeVisible();
  await expect(connections.getByText('This device Bridge')).toBeVisible();
  await expect(connections.getByText('Agent runtimes')).toBeVisible();
  await expect(connections.getByText('Not installed')).toBeVisible();
  await expect(page.getByText('Connection diagnostics')).toBeVisible();
  await expect(page.getByText('2 service layers need attention.')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Rooms' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('窄屏状态与诊断布局没有横向溢出且可以折叠', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  const diagnostics = page.locator('.workspace-diagnostics');
  await expect(diagnostics).toHaveAttribute('open', '');
  await diagnostics.locator('summary').click();
  await expect(diagnostics).not.toHaveAttribute('open', '');
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

function collectPageFailures(page: Page): string[] {
  const failures: string[] = [];
  page.on('pageerror', (error) => failures.push(error.message));
  page.on('console', (message) => {
    if (message.type() === 'error') failures.push(message.text());
  });
  return failures;
}

async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  const dimensions = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
}
