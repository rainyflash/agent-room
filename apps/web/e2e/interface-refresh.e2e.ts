import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const surfaces = [
  ['rooms', '/e2e/fixtures/room-directory.html'],
  ['agents', '/e2e/fixtures/account-workspace.html'],
  ['security', '/e2e/fixtures/security-center.html'],
  ['my-agents-desktop', '/e2e/fixtures/my-agents.html'],
  ['settings-general', '/e2e/fixtures/my-agents.html?settings=general'],
] as const;

for (const width of [1440, 390]) {
  for (const [name, path] of surfaces) {
    test(`统一界面 ${name}：${String(width)}px 布局和无障碍`, async ({ page }, testInfo) => {
      const failures = collectPageFailures(page);
      await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto(path);
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      if (name === 'my-agents-desktop')
        await expect(page.getByText('Studio companion', { exact: true })).toBeVisible();
      await expect(page.getByRole('navigation', { name: 'Explore Agent Room' })).toBeVisible();
      await expectNoHorizontalOverflow(page);
      const scan = await new AxeBuilder({ page })
        .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
        .analyze();
      expect(
        scan.violations.map(({ id, nodes }) => ({
          id,
          nodes: nodes.map(({ target, failureSummary }) => ({ target, failureSummary })),
        })),
      ).toEqual([]);
      expect(failures).toEqual([]);
      await page.screenshot({
        animations: 'disabled',
        fullPage: true,
        path: testInfo.outputPath(`${name}-${String(width)}.png`),
      });
    });
  }
  test(`设置 · 这台电脑：${String(width)}px 有新版本、自动打开与更新`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/e2e/fixtures/my-agents.html?settings=this-computer');
    await expect(page.getByRole('heading', { level: 1, name: 'Settings' })).toBeVisible();
    await expect(page.getByRole('link', { name: 'This computer: Connected' })).toBeVisible();
    // 启动后自动查到新版本：提示栈里一条去安装，“设置”和“这台电脑”上都有提醒点。
    await expect(page.getByText('Agent Room 0.1.0-alpha.24 is ready to install')).toBeVisible();
    await expect(page.getByRole('link', { name: /^Settings.*Update ready/u })).toBeVisible();
    const content = page.getByRole('region', { name: 'This computer', exact: true });
    await content.getByRole('button', { name: 'Off', exact: true }).click();
    await expect(content.getByRole('button', { name: 'On', exact: true })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(content.getByText('0.1.0-alpha.23 → 0.1.0-alpha.24')).toBeVisible();
    await expect(content.getByRole('button', { name: 'Install and restart' })).toBeVisible();
    await expectNoHorizontalOverflow(page);
    const scan = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(
      scan.violations.map(({ id, nodes }) => ({
        id,
        nodes: nodes.map(({ target, failureSummary }) => ({ target, failureSummary })),
      })),
    ).toEqual([]);
    expect(failures).toEqual([]);
    await page.screenshot({
      animations: 'disabled',
      fullPage: true,
      path: testInfo.outputPath(`settings-this-computer-${String(width)}.png`),
    });
  });
}
test('中文窄屏安全操作不挤成竖排，身份与授权入口保持可用', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(() => {
    localStorage.setItem('agent-room.language', 'zh-CN');
  });
  await page.goto('/e2e/fixtures/security-center.html');
  const refresh = page.getByRole('button', { name: '刷新', exact: true });
  await expect(refresh).toBeVisible();
  expect((await refresh.boundingBox())?.height).toBeGreaterThanOrEqual(44);
  expect((await refresh.boundingBox())?.height).toBeLessThan(80);
  await refresh.click();
  await expect(page.getByRole('heading', { name: '电脑和浏览器' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath('security-zh-390.png'),
  });
});
