import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

// 网页通知（specs/redesign/reminders.md 4R-2）：设置“通用”里“提醒”一组，只在网页上有这一行。
const web = '/e2e/fixtures/my-agents.html?browser&settings=general';
const desktop = '/e2e/fixtures/my-agents.html?settings=general';

/**
 * 浏览器自己的通知权限各处不一样：Linux 上的无头 Chromium 不管给没给权限都说“拒绝”。用例换成假的
 * `Notification`，从给定的权限开始；问的时候答“允许”，和真的浏览器一样记着，重开页面还是允许。
 */
async function fakeNotifications(page: Page, permission: 'default' | 'granted' | 'denied') {
  await page.addInitScript((initial) => {
    const key = 'e2e.fake-notification-permission';
    let current = sessionStorage.getItem(key) ?? initial;
    class FakeNotification {
      onclick: (() => void) | null = null;
      static get permission() {
        return current;
      }
      static requestPermission() {
        current = 'granted';
        sessionStorage.setItem(key, current);
        return Promise.resolve(current);
      }
      close() {
        this.onclick = null;
      }
    }
    Object.defineProperty(window, 'Notification', { configurable: true, value: FakeNotification });
  }, permission);
}

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
  test(`网页上打开网页通知，重开页面还开着 ${String(width)}px`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await fakeNotifications(page, 'default');
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

test('服务器开了邮件提醒才有那一行，改了重开页面还是改过的；暂停的说明提到邮件', async ({
  page,
}, testInfo) => {
  const failures = collectPageFailures(page);
  await fakeNotifications(page, 'default');
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto(web);
  await expect(page.getByRole('switch', { name: 'Web notifications' })).toBeVisible();
  await expect(page.getByRole('switch', { name: 'Email reminders' })).toHaveCount(0);
  await expect(page.getByText(/no reminder emails/u)).toHaveCount(0);

  await page.goto(`${web}&email=on`);
  const email = page.getByRole('switch', { name: 'Email reminders' });
  await expect(email).toBeChecked();
  await expect(email).toHaveAccessibleDescription(
    /At most one a day, never the messages themselves\./u,
  );
  await expect(page.getByText(/no reminder emails go out/u)).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath('email-reminders-390.png'),
  });
  await email.click();
  await expect(email).not.toBeChecked();
  await page.reload();
  await expect(page.getByRole('switch', { name: 'Email reminders' })).not.toBeChecked();

  // 邮件提醒跟着账户走，桌面端也有这一行。
  await page.goto(`${desktop}&email=off`);
  await expect(page.getByRole('switch', { name: 'Email reminders' })).not.toBeChecked();
  expect(failures).toEqual([]);
});

test('中文：提醒一组的字；桌面端没有网页通知这一行', async ({ browser }) => {
  const context = await browser.newContext({ locale: 'zh-CN' });
  try {
    const page = await context.newPage();
    await fakeNotifications(page, 'denied');
    await page.goto(web);
    await expect(page.getByRole('heading', { level: 3, name: '提醒' })).toBeVisible();
    // 浏览器拒过：开关灰着，照实说去哪儿改。
    const toggle = page.getByRole('switch', { name: '网页通知' });
    await expect(toggle).toBeDisabled();
    await expect(toggle).toHaveAccessibleDescription(/把“通知”改成允许/u);
    await expect(page.getByRole('button', { name: '暂停一小时' })).toBeVisible();
    await expect(page.getByRole('switch', { name: '邮件提醒' })).toHaveCount(0);

    await page.goto(`${web}&email=on`);
    await expect(page.getByRole('switch', { name: '邮件提醒' })).toHaveAccessibleDescription(
      /每天最多一封，不写消息内容。/u,
    );
    await expect(
      page.getByText('新消息照样进收件箱，只是不弹出来，也不发提醒邮件。'),
    ).toBeVisible();

    await page.goto(desktop);
    await expect(page.getByRole('heading', { level: 3, name: '提醒' })).toBeVisible();
    await expect(page.getByRole('switch', { name: '网页通知' })).toHaveCount(0);
  } finally {
    await context.close();
  }
});
