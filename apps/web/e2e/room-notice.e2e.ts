import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openRoomSettings } from './support/workspace-navigation';

const roomFixture = '/e2e/fixtures/lobby-scene.html';

for (const width of [1440, 390]) {
  test(`房间公告：开头两行、没看过的亮点，点开看全文；房主在设置里改、撤掉：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private&notice&view=conversation`);

    const notice = page.getByRole('region', { name: 'Room notice' });
    await expect(notice).toContainText('Only speak when someone mentions you.');
    await expect(notice.getByText('Updated since you last looked')).toBeAttached();
    const text = notice.locator('.room-notice__text');
    // 三行的公告平时只显示两行。
    const clamped = await text.evaluate((element) => element.scrollHeight > element.clientHeight);
    expect(clamped).toBe(true);
    await expectNoHorizontalOverflow(page);
    const scan = await new AxeBuilder({ page })
      .include('.room-notice')
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(scan.violations.map(({ id }) => id)).toEqual([]);

    await notice.getByRole('button', { name: /Show the whole notice/u }).click();
    await expect(notice).toContainText('Ask Grace before changing the release checklist.');
    await expect(notice).toContainText(/Updated by Grace · /u);
    await expect(notice.getByText('Updated since you last looked')).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath(`room-notice-${String(width)}.png`) });

    // 看过的记在本机：重新打开不再亮。
    await page.reload();
    await expect(page.getByRole('region', { name: 'Room notice' })).toBeVisible();
    await expect(page.getByText('Updated since you last looked')).toHaveCount(0);

    await page.getByRole('button', { name: 'Edit notice' }).click();
    const dialog = page.getByRole('dialog', { name: 'Room settings' });
    await expect(dialog.getByRole('radio', { name: 'Notice' })).toBeChecked();
    const field = dialog.getByRole('textbox', { name: 'Notice' });
    await expect(field).toHaveValue(/^Only speak when someone mentions you\./u);
    await expect(dialog).toContainText('Notices aren’t encrypted');
    await field.fill('Reply in English.');
    await dialog.getByRole('button', { name: 'Save notice' }).click();
    await expect(dialog).toContainText('Notice updated.');
    await expectNoHorizontalOverflow(page);
    await dialog.getByRole('button', { name: 'Close room settings' }).click();
    await expect(page.getByRole('region', { name: 'Room notice' })).toContainText(
      'Reply in English.',
    );

    const settings = await openRoomSettings(page, 'Notice');
    await settings.getByRole('button', { name: 'Remove notice' }).click();
    await expect(settings).toContainText('Notice removed.');
    await settings.getByRole('button', { name: 'Close room settings' }).click();
    await expect(page.getByRole('region', { name: 'Room notice' })).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}

test('没有公告的房间：对话上方什么都不多，房主在设置的“公告”里写第一条', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${roomFixture}?private&view=conversation`);
  await expect(page.getByRole('log', { name: /conversation/iu })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Room notice' })).toHaveCount(0);

  const dialog = await openRoomSettings(page, 'Notice');
  await expect(dialog.getByRole('button', { name: 'Remove notice' })).toHaveCount(0);
  await dialog.getByRole('textbox', { name: 'Notice' }).fill('Be kind.\nStay on topic.');
  await dialog.getByRole('button', { name: 'Save notice' }).click();
  await expect(dialog).toContainText('Notice updated.');
  await dialog.getByRole('button', { name: 'Close room settings' }).click();

  const notice = page.getByRole('region', { name: 'Room notice' });
  await expect(notice).toContainText('Be kind.');
  // 自己写的不算没看过。
  await expect(notice.getByText('Updated since you last looked')).toHaveCount(0);
  expect(failures).toEqual([]);
});
