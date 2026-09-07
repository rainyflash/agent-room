import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const surfaces = [
  ['rooms', '/e2e/fixtures/room-directory.html'],
  ['agents', '/e2e/fixtures/account-workspace.html'],
  ['security', '/e2e/fixtures/security-center.html'],
  ['onboarding', '/e2e/fixtures/onboarding.html?browser'],
] as const;

for (const width of [1440, 390]) {
  for (const [name, path] of surfaces) {
    test(`统一界面 ${name}：${String(width)}px 布局和无障碍`, async ({ page }, testInfo) => {
      const failures = collectPageFailures(page);
      await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.goto(path);
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      if (name === 'onboarding')
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
  test(`桌面设置：${String(width)}px 展开、更新与自动启动`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/e2e/fixtures/onboarding.html');
    const trigger = page.getByRole('button', { name: /This computer/u });
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'true');
    const update = page.getByRole('button', { name: 'Check', exact: true });
    await update.click();
    await expect(page.getByText('0.1.0-alpha.24', { exact: false })).toBeVisible();
    const autostart = page.getByRole('button', { name: 'Off', exact: true });
    await autostart.click();
    await expect(page.getByRole('button', { name: 'On', exact: true })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
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
      path: testInfo.outputPath(`desktop-settings-${String(width)}.png`),
    });
    await trigger.click();
    await expect(trigger).toHaveAttribute('aria-expanded', 'false');
  });
}
test('中文窄屏安全操作不挤成竖排，身份与授权入口保持可用', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(() => {
    localStorage.setItem('agent-room.language', 'zh-CN');
  });
  await page.goto('/e2e/fixtures/security-center.html');
  const refresh = page.getByRole('button', { name: '刷新访问状态' });
  await expect(refresh).toBeVisible();
  expect((await refresh.boundingBox())?.height).toBeGreaterThanOrEqual(44);
  expect((await refresh.boundingBox())?.height).toBeLessThan(80);
  await refresh.click();
  await expect(page.getByRole('heading', { name: '产品设备' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath('security-zh-390.png'),
  });
});
