import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import type { MyAgentsFixtureWindow } from '../src/test/my-agents-fixture-controls';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixture = '/e2e/fixtures/my-agents.html';

async function expectAccessible(page: Page) {
  const scan = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({
      id,
      nodes: nodes.map(({ target, failureSummary }) => ({ target, failureSummary })),
    })),
  ).toEqual([]);
}

for (const width of [1440, 390]) {
  test(`查到新版本：提示上一步更新，“稍后”重开也记得，托盘的请求照样显示进度 ${String(width)}px`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto(fixture);
    const toasts = page.getByRole('region', { name: 'Notifications' });
    await expect(toasts.getByText('Agent Room 0.1.0-alpha.24 is ready to install')).toBeVisible();
    await expect(toasts.getByRole('button', { name: 'Update and restart' })).toBeEnabled();
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page);
    await page.screenshot({
      animations: 'disabled',
      path: testInfo.outputPath(`update-toast-${String(width)}.png`),
    });

    // 稍后：这一版 24 小时内不再提醒，重开页面也记得；“设置”上的提醒点还在。
    await toasts.getByRole('button', { name: 'Later' }).click();
    await expect(toasts.getByText('Agent Room 0.1.0-alpha.24 is ready to install')).toHaveCount(0);
    await page.reload();
    await expect(page.getByRole('link', { name: /^Settings.*Update ready/u })).toBeVisible();
    await expect(toasts.getByText('Agent Room 0.1.0-alpha.24 is ready to install')).toHaveCount(0);

    // 托盘菜单“更新到 0.1.0-alpha.24…”：提示出来，按钮上是下载进度。
    await page.evaluate(() => {
      (window as MyAgentsFixtureWindow).__agentRoomFixtureControls.requestUpdate();
    });
    await expect(toasts.getByRole('button', { name: 'Downloading 50%' })).toBeDisabled();
    expect(failures).toEqual([]);
  });
}

test('中文：标题和按钮跟系统通知里说的一样', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('agent-room.language', 'zh-CN');
  });
  await page.goto(fixture);
  const toasts = page.getByRole('region', { name: 'Notifications' });
  await expect(toasts.getByText('新版本 0.1.0-alpha.24 可以安装了')).toBeVisible();
  await expect(toasts.getByRole('button', { name: '更新并重启' })).toBeVisible();
});

test('设置里写上次检查：已是最新、没查成、清单过期', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${fixture}?settings=this-computer&update=current`);
  const content = page.getByRole('region', { name: 'This computer', exact: true });
  await expect(content.getByText(/^Last checked .+: you’re up to date\.$/u)).toBeVisible();
  await expect(page.getByText(/ready to install/u)).toHaveCount(0);

  await page.goto(`${fixture}?settings=this-computer&update=failed`);
  await expect(content.getByText(/the check didn’t go through/u)).toBeVisible();
  await content.getByText('Details').click();
  await expect(content.getByText('desktop.update.manifest_network')).toBeVisible();
  await expectAccessible(page);

  await page.goto(`${fixture}?settings=this-computer&update=expired`);
  await expect(content.getByText(/This clears up with the next release\./u)).toBeVisible();
  await expect(content.getByText('desktop.update.manifest_expired')).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('Mac 上应用在只读位置运行：提示和设置都说先拖进“应用程序”文件夹，不给更新按钮', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.goto(`${fixture}?update=translocated`);
  const toasts = page.getByRole('region', { name: 'Notifications' });
  await expect(toasts.getByText(/Move it into the Applications folder/u)).toBeVisible();
  await expect(toasts.getByRole('button', { name: 'Update and restart' })).toHaveCount(0);

  await page.goto(`${fixture}?settings=this-computer&update=translocated`);
  const content = page.getByRole('region', { name: 'This computer', exact: true });
  await expect(content.getByText(/Move it into the Applications folder/u)).toBeVisible();
  await expect(content.getByRole('button', { name: 'Update and restart' })).toHaveCount(0);
  await expectAccessible(page);
  expect(failures).toEqual([]);
});
