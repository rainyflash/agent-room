import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const surfaces = [
  ['rooms', '/e2e/fixtures/room-directory.html'],
  ['agents', '/e2e/fixtures/account-workspace.html'],
  ['security', '/e2e/fixtures/security-center.html'],
  ['my-agents-desktop', '/e2e/fixtures/my-agents.html'],
  ['my-agents-browser', '/e2e/fixtures/my-agents.html?browser'],
  ['settings-general', '/e2e/fixtures/my-agents.html?settings=general'],
  ['settings-account', '/e2e/fixtures/my-agents.html?settings=account'],
  ['settings-about-browser', '/e2e/fixtures/my-agents.html?browser&settings=about'],
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
      // 网页端登录以后也找得到下载和 GitHub：“我的 Agent”里有桌面应用一节，“关于”里有源代码。
      if (name === 'my-agents-browser') {
        const app = page.getByRole('region', { name: 'Desktop app', exact: true });
        await expect(app.getByRole('link', { name: 'All versions on GitHub' })).toBeVisible();
        await expect(page.getByRole('region', { name: 'This computer', exact: true })).toHaveCount(
          0,
        );
      }
      if (name === 'settings-about-browser')
        await expect(page.getByRole('link', { name: 'Source code on GitHub' })).toBeVisible();
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
  test(`设置 · 账户：${String(width)}px 删除账户要勾选并输入 DELETE`, async ({ page }) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    // 网页端：桌面端夹具会弹“有新版本”的提示，在手机宽度上盖住按钮。
    await page.goto('/e2e/fixtures/my-agents.html?browser&settings=account');
    const content = page.getByRole('region', { name: 'Account', exact: true });
    await expect(content.getByRole('button', { name: 'Download', exact: true })).toBeVisible();
    await content.getByRole('button', { name: 'Delete account…' }).click();
    const confirm = content.getByRole('button', { name: 'Delete my account' });
    await expect(confirm).toBeDisabled();
    await content.getByRole('checkbox').check();
    await content.getByRole('textbox', { name: 'Type DELETE to confirm' }).fill('DELETE');
    await expect(confirm).toBeEnabled();
    await expectNoHorizontalOverflow(page);
    const scan = await new AxeBuilder({ page })
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(scan.violations.map(({ id }) => id)).toEqual([]);
    expect(failures).toEqual([]);
  });
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

test('消息没连上时提示栈里常驻一条：登录超时给“重新连接”，等浏览器登录时能重新开始', async ({
  page,
}, testInfo) => {
  const failures = collectPageFailures(page);
  const notifications = page.getByRole('region', { name: 'Notifications' });
  await page.goto('/e2e/fixtures/my-agents.html?matrix=failed');
  // 生产上“我的 Agent”页只看控制面，消息没连上时照样显示。
  await expect(page.getByText('Studio companion', { exact: true })).toBeVisible();
  await expect(notifications.getByText('Messages are not connected')).toBeVisible();
  await expect(
    notifications.getByText(
      'The previous connection was not completed. Choose Reconnect to continue.',
    ),
  ).toBeVisible();
  const scan = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(scan.violations.map(({ id }) => id)).toEqual([]);
  await page.screenshot({
    animations: 'disabled',
    path: testInfo.outputPath('matrix-connection-failed.png'),
  });
  // 夹具里每次登录都超时，重新连接一遍后还是停在这条提示上。
  await notifications.getByRole('button', { name: 'Reconnect' }).click();
  await expect(notifications.getByText('Messages are not connected')).toBeVisible();

  await page.goto('/e2e/fixtures/my-agents.html?matrix=signin');
  await expect(notifications.getByText('Finish signing in in your browser')).toBeVisible();
  await notifications.getByRole('button', { name: 'Start sign-in again' }).click();
  await expect(notifications.getByText('Finish signing in in your browser')).toBeVisible();
  expect(failures).toEqual([]);
});
