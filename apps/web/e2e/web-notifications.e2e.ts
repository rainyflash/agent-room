import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

// 网页通知（specs/redesign/reminders.md 4R-2）：设置“通用”里“提醒”一组，只在网页上有这一行。
const web = '/e2e/fixtures/my-agents.html?browser&settings=general';
const desktop = '/e2e/fixtures/my-agents.html?settings=general';

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
  test(`网页上打开网页通知，重开页面还开着 ${String(width)}px`, async ({
    context,
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await context.grantPermissions(['notifications']);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto(web);
    await expect(page.getByRole('heading', { level: 3, name: 'Reminders' })).toBeVisible();
    const toggle = page.getByRole('switch', { name: 'Web notifications' });
    await expect(toggle).not.toBeChecked();
    await expect(toggle).toHaveAccessibleDescription(/Only in this browser\./u);
    await toggle.click();
    await expect(toggle).toBeChecked();
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page);
    await page.screenshot({
      animations: 'disabled',
      fullPage: true,
      path: testInfo.outputPath(`web-notifications-${String(width)}.png`),
    });

    // 记在这个浏览器里：重开页面还开着；关上也记得。
    await page.reload();
    await expect(page.getByRole('switch', { name: 'Web notifications' })).toBeChecked();
    await page.getByRole('switch', { name: 'Web notifications' }).click();
    await page.reload();
    await expect(page.getByRole('switch', { name: 'Web notifications' })).not.toBeChecked();
    expect(failures).toEqual([]);
  });
}

test('中文：提醒一组的字；桌面端没有网页通知这一行', async ({ browser }) => {
  const context = await browser.newContext({ locale: 'zh-CN' });
  try {
    const page = await context.newPage();
    await page.goto(web);
    await expect(page.getByRole('heading', { level: 3, name: '提醒' })).toBeVisible();
    const toggle = page.getByRole('switch', { name: '网页通知' });
    await expect(toggle).toHaveAccessibleDescription(/只管这个浏览器。/u);
    await expect(page.getByRole('button', { name: '暂停一小时' })).toBeVisible();

    await page.goto(desktop);
    await expect(page.getByRole('heading', { level: 3, name: '提醒' })).toBeVisible();
    await expect(page.getByRole('switch', { name: '网页通知' })).toHaveCount(0);
  } finally {
    await context.close();
  }
});
