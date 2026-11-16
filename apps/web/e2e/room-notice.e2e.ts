import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openRoomSettings } from './support/workspace-navigation';

const roomFixture = '/e2e/fixtures/lobby-scene.html';

for (const width of [1440, 390]) {
  test(`房间须知：开头两行、没看过的亮点，点开看全文；房主在设置里改、撤掉，对话里点开看每一版：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private&notice&view=conversation`);

    const notice = page.getByRole('region', { name: 'Room brief' });
    const log = page.getByRole('log', { name: /conversation/iu });
    await expect(notice).toContainText('Only speak when someone mentions you.');
    await expect(notice.getByText('Changed since you last looked')).toBeAttached();
    // 对话里按时间写着这一次改动。
    await expect(log.locator('.room-notice-change')).toHaveText([/Grace changed the brief/u]);
    const text = notice.locator('.room-notice__text');
    // 三行的须知平时只显示两行。
    const clamped = await text.evaluate((element) => element.scrollHeight > element.clientHeight);
    expect(clamped).toBe(true);
    await expectNoHorizontalOverflow(page);
    const scan = await new AxeBuilder({ page })
      .include('.room-notice')
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(scan.violations.map(({ id }) => id)).toEqual([]);

    await notice.getByRole('button', { name: /Show the whole brief/u }).click();
    await expect(notice).toContainText('Ask Grace before changing the release checklist.');
    await expect(notice).toContainText(/Changed by Grace · /u);
    await expect(notice.getByText('Changed since you last looked')).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath(`room-notice-${String(width)}.png`) });

    // 看过的记在个人工作区：重新打开不再亮。
    await page.reload();
    await expect(page.getByRole('region', { name: 'Room brief' })).toBeVisible();
    await expect(page.getByText('Changed since you last looked')).toHaveCount(0);

    await page.getByRole('button', { name: 'Edit brief' }).click();
    const dialog = page.getByRole('dialog', { name: 'Room settings' });
    await expect(dialog.getByRole('radio', { name: 'Brief' })).toBeChecked();
    const field = dialog.getByRole('textbox', { name: 'Brief' });
    await expect(field).toHaveValue(/^Only speak when someone mentions you\./u);
    await expect(dialog).toContainText('The brief isn’t encrypted');
    await field.fill('Reply in English.');
    await dialog.getByRole('button', { name: 'Save brief' }).click();
    await expect(dialog).toContainText('Brief updated.');
    await expectNoHorizontalOverflow(page);
    await dialog.getByRole('button', { name: 'Close room settings' }).click();
    await expect(page.getByRole('region', { name: 'Room brief' })).toContainText(
      'Reply in English.',
    );

    const settings = await openRoomSettings(page, 'Brief');
    await settings.getByRole('button', { name: 'Remove brief' }).click();
    await expect(settings).toContainText('Brief removed.');
    await settings.getByRole('button', { name: 'Close room settings' }).click();
    await expect(page.getByRole('region', { name: 'Room brief' })).toHaveCount(0);
    // 重新加载以后改动还按 Grace 那一次算；这一页里改的两次跟在后面。
    await expect(log.locator('.room-notice-change')).toHaveText([
      /Grace changed the brief/u,
      /You changed the brief/u,
      /You removed the brief/u,
    ]);
    // 每一次改动点开看那一版：Grace 那一行是改之前的，撤掉的那一行没有可看的。
    const versions = log.getByRole('button', { name: 'Show this version' });
    await expect(versions).toHaveCount(2);
    await versions.first().click();
    const graceVersion = log.locator('.room-notice-change__version').first();
    await expect(graceVersion).toBeVisible();
    await expect(graceVersion).toContainText('Ask Grace before changing the release checklist.');
    await expect(log.getByRole('button', { name: 'Hide' })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    const versionScan = await new AxeBuilder({ page })
      .include('.room-notice-change')
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(versionScan.violations.map(({ id }) => id)).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath(`room-brief-version-${String(width)}.png`) });
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
  });
}

test('在别的设备上看过这一版须知：这个浏览器里没记过也不亮；中文叫须知', async ({
  browser,
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.goto(`${roomFixture}?private&notice&seenElsewhere&view=conversation`);
  const notice = page.getByRole('region', { name: 'Room brief' });
  await expect(notice).toContainText('Only speak when someone mentions you.');
  await expect(notice.getByText('Changed since you last looked')).toHaveCount(0);

  const context = await browser.newContext({ locale: 'zh-CN' });
  const chinese = await context.newPage();
  await chinese.goto(`${roomFixture}?private&notice&view=conversation`);
  const brief = chinese.getByRole('region', { name: '房间须知' });
  await expect(brief).toContainText('须知');
  await expect(brief.getByText('改过了，你还没看')).toBeAttached();
  await expect(chinese.locator('.room-notice-change')).toHaveText([/Grace 改了须知/u]);
  await chinese.getByRole('button', { name: '看这一版' }).click();
  await expect(chinese.locator('.room-notice-change__version')).toContainText('这一版');
  await context.close();
  expect(failures).toEqual([]);
});

test('以前的房主删除了账户：对话里写房间交给了谁，中英文', async ({ browser, page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${roomFixture}?private&handover&view=conversation`);
  const log = page.getByRole('log', { name: /conversation/iu });
  await expect(log.locator('.room-notice-change')).toHaveText([
    /The owner deleted their account, so this room now belongs to Grace/u,
  ]);
  // 没有须知：对话上方不多出须知那一条。
  await expect(page.getByRole('region', { name: 'Room brief' })).toHaveCount(0);
  expect(failures).toEqual([]);

  const context = await browser.newContext({ locale: 'zh-CN' });
  const chinese = await context.newPage();
  await chinese.goto(`${roomFixture}?private&handover&view=conversation`);
  await expect(chinese.locator('.room-notice-change')).toHaveText([
    /房主的账户已经删除，房间交给了 Grace/u,
  ]);
  await context.close();
});

test('没有须知的房间：对话上方什么都不多，房主在设置的“须知”里写第一条', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${roomFixture}?private&view=conversation`);
  await expect(page.getByRole('log', { name: /conversation/iu })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Room brief' })).toHaveCount(0);

  const dialog = await openRoomSettings(page, 'Brief');
  await expect(dialog.getByRole('button', { name: 'Remove brief' })).toHaveCount(0);
  await dialog.getByRole('textbox', { name: 'Brief' }).fill('Be kind.\nStay on topic.');
  await dialog.getByRole('button', { name: 'Save brief' }).click();
  await expect(dialog).toContainText('Brief updated.');
  await dialog.getByRole('button', { name: 'Close room settings' }).click();

  const notice = page.getByRole('region', { name: 'Room brief' });
  await expect(notice).toContainText('Be kind.');
  // 自己写的不算没看过。
  await expect(notice.getByText('Changed since you last looked')).toHaveCount(0);
  await expect(
    page.getByRole('log', { name: /conversation/iu }).locator('.room-notice-change'),
  ).toHaveText([/You changed the brief/u]);
  expect(failures).toEqual([]);
});
