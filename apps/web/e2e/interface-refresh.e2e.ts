import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
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
        await expect(
          page
            .getByRole('region', { name: 'Agent Room', exact: true })
            .getByRole('link', { name: 'Source code on GitHub' }),
        ).toBeVisible();
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
  test(`设置 · 账户：${String(width)}px 退出、换账号，删除账户要勾选并输入 DELETE`, async ({
    page,
  }) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    // 网页端：桌面端夹具会弹“有新版本”的提示，在手机宽度上盖住按钮。
    await page.goto('/e2e/fixtures/my-agents.html?browser&settings=account');
    const content = page.getByRole('region', { name: 'Account', exact: true });
    // 退出登录就在登录的账户旁边，网页上还能换个账号（specs/sign-out/design.md）。
    await expect(content.getByRole('button', { name: 'Sign out', exact: true })).toBeVisible();
    await expect(
      content.getByRole('button', { name: 'Switch account', exact: true }),
    ).toBeVisible();
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
    // 原生层查到新版本：提示栈里一条，“设置”和“这台电脑”上都有提醒点。
    await expect(page.getByText('Agent Room 0.1.0-alpha.24 is ready to install')).toBeVisible();
    await expect(page.getByRole('link', { name: /^Settings.*Update ready/u })).toBeVisible();
    const content = page.getByRole('region', { name: 'This computer', exact: true });
    await content.getByRole('button', { name: 'Off', exact: true }).click();
    await expect(content.getByRole('button', { name: 'On', exact: true })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await expect(content.getByText('0.1.0-alpha.23 → 0.1.0-alpha.24')).toBeVisible();
    await expect(content.getByRole('button', { name: 'Update and restart' })).toBeVisible();
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

test('消息没连上时提示栈里常驻一条：聊天登录没拿到给“重新连接”', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  const notifications = page.getByRole('region', { name: 'Notifications' });
  await page.goto('/e2e/fixtures/my-agents.html?matrix=failed');
  // 生产上“我的 Agent”页只看控制面，消息没连上时照样显示。
  await expect(page.getByText('Studio companion', { exact: true })).toBeVisible();
  await expect(notifications.getByText('Messages are not connected')).toBeVisible();
  await expect(
    notifications.getByText('Your conversations could not connect. Please retry.'),
  ).toBeVisible();
  const scan = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(scan.violations.map(({ id }) => id)).toEqual([]);
  await page.screenshot({
    animations: 'disabled',
    path: testInfo.outputPath('matrix-connection-failed.png'),
  });
  // 夹具里每次都拿不到聊天登录，重新连接一遍后还是停在这条提示上。
  await notifications.getByRole('button', { name: 'Reconnect' }).click();
  await expect(notifications.getByText('Messages are not connected')).toBeVisible();

  expect(failures).toEqual([]);
});

const MAC_USER_AGENT =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36';
const MAC_INSTALLER = 'https://download.fixture.invalid/agent-room.dmg';

// 网页登录以后也一眼找得到 GitHub 和下载：顶栏右边，地方不够时收成图标，名字、标志和四个去处
// 都不被挤；桌面端顶栏没有这两样。
for (const width of [1440, 1024, 390]) {
  test(`网页顶栏：${String(width)}px 有 GitHub 和下载应用，桌面端没有`, async ({ browser }) => {
    const context = await browser.newContext({
      reducedMotion: 'reduce',
      userAgent: MAC_USER_AGENT,
      viewport: { width, height: width === 390 ? 844 : 1000 },
    });
    // 开发服务器的运行时清单里没有安装包地址，这里补一个 Mac 的。
    await context.route('**/runtime-manifest.json', async (route) => {
      const response = await route.fetch();
      const manifest = (await response.json()) as Record<string, unknown>;
      await route.fulfill({ response, json: { ...manifest, macosDownloadUrl: MAC_INSTALLER } });
    });
    const page = await context.newPage();
    const failures = collectPageFailures(page);
    const header = page.locator('.app-navigation');

    await page.goto('/e2e/fixtures/my-agents.html?browser');
    await expect(header.getByRole('link', { name: 'Source code on GitHub' })).toHaveAttribute(
      'href',
      'https://github.com/rainyflash/agent-room',
    );
    const download = header.getByRole('link', { name: 'Download app' });
    await expect(download).toBeVisible();
    await expect(download).toHaveAttribute('href', MAC_INSTALLER);
    await expect(download).toHaveAttribute('title', 'Download for Mac');
    expect(await crowdedHeaderParts(page)).toEqual([]);
    await expectNoHorizontalOverflow(page);
    const scan = await new AxeBuilder({ page })
      .include('.app-navigation')
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(scan.violations.map(({ id }) => id)).toEqual([]);

    await page.goto('/e2e/fixtures/my-agents.html');
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
    await expect(header.getByRole('link', { name: 'Source code on GitHub' })).toHaveCount(0);
    await expect(header.getByRole('link', { name: 'Download app' })).toHaveCount(0);
    expect(await crowdedHeaderParts(page)).toEqual([]);
    expect(failures).toEqual([]);
    await context.close();
  });
}

/** 顶栏里被挤得放不下的部分：名字和标志、四个去处。 */
async function crowdedHeaderParts(page: Page): Promise<string[]> {
  return await page.locator('.app-navigation').evaluate((header) =>
    ['.app-navigation__identity', '.app-navigation__links'].filter((selector) => {
      const part = header.querySelector(selector);
      return part !== null && part.scrollWidth > part.clientWidth;
    }),
  );
}
