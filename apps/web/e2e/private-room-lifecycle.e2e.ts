import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { openRoomMenu, openRoomSettings } from './support/workspace-navigation';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html';

async function expectAccessibleDialog(page: Page): Promise<void> {
  const scan = await new AxeBuilder({ page })
    .include('.ar-dialog')
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({ id, targets: nodes.map(({ target }) => target) })),
  ).toEqual([]);
}
const first = '0198b601-77a2-7f41-b4f4-940f291951b8';
const second = '0198b601-77a2-7f41-b4f4-940f291951b9';

test('新建房间一屏：保留多久、现在就邀请谁放在更多选项里', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  await openRoomMenu(page);
  await page.getByRole('button', { name: 'New room', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'New room' });
  await expect(dialog).toContainText('Private rooms are invite only');
  await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Architecture review');
  await dialog
    .getByRole('textbox', { name: 'Purpose (optional)' })
    .fill('Coordinate a bounded design review.');

  await dialog.getByText('More options').click();
  await dialog.getByRole('combobox', { name: 'Keep messages for' }).selectOption('30');
  await dialog
    .getByRole('textbox', { name: 'Invite people now (optional)' })
    .fill(`${first}\n${second}`);
  await dialog.getByRole('checkbox', { name: /Automate/ }).check();
  await expect(dialog.getByRole('checkbox', { name: /Speak/ })).toBeChecked();

  await dialog.getByRole('button', { name: 'Create and enter' }).click();
  await expect(dialog.getByRole('alert')).toContainText('Could not finish creating the room');
  await expect(dialog).toContainText('private_room.fixture_unavailable');
  await expect(dialog.getByRole('textbox', { name: 'Name', exact: true })).toBeDisabled();
  await expectAccessibleDialog(page);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath('new-room.png'),
  });
});

test('新建房间在手机上是底部面板，不产生横向溢出', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await openRoomMenu(page);
  await page.getByRole('button', { name: 'New room', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'New room' });
  await expect(dialog).toBeVisible();
  await dialog.getByText('More options').click();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

for (const width of [1_440, 390]) {
  test(`私人房间的房间设置：四节在一个对话框里 ${String(width)}`, async ({ page }) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private`);
    await expect(page.locator('.workspace-header__identity')).toContainText('Private room');

    const dialog = await openRoomSettings(page);
    const sections = dialog.getByRole('radiogroup', { name: 'Room settings sections' });
    await expect(sections.getByRole('radio')).toHaveText([
      'Members',
      'Agent code',
      'Automation',
      'Moderation',
    ]);
    await expect(dialog.getByRole('textbox', { name: 'Room name' })).toHaveValue(
      'Builders Exchange',
    );
    // 房间 ID 这类排查信息收在详情里。
    await expect(dialog.getByText('Room details')).toBeVisible();
    await dialog.getByRole('textbox', { name: 'Account ID' }).fill('0198b601');
    await sections.getByRole('radio', { name: 'Agent code' }).click();
    await expect(dialog.getByRole('button', { name: 'Create code' })).toBeVisible();
    await sections.getByRole('radio', { name: 'Members' }).click();
    await expect(dialog.getByRole('textbox', { name: 'Account ID' })).toHaveValue('0198b601');
    await expectAccessibleDialog(page);
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
  });
}
