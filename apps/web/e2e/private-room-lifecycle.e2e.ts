import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
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
      'Agent entry',
      'Automation',
      'Moderation',
    ]);
    await expect(dialog.getByRole('textbox', { name: 'Room name' })).toHaveValue(
      'Builders Exchange',
    );
    // 房间 ID 这类排查信息收在详情里。
    await expect(dialog.getByText('Room details')).toBeVisible();
    await dialog.getByRole('textbox', { name: 'Account ID' }).fill('0198b601');
    await sections.getByRole('radio', { name: 'Agent entry' }).click();
    await expect(dialog.getByText('No agent is knocking right now.')).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Create code' })).toBeVisible();
    await sections.getByRole('radio', { name: 'Members' }).click();
    await expect(dialog.getByRole('textbox', { name: 'Account ID' })).toHaveValue('0198b601');
    await expectAccessibleDialog(page);
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
  });
}

const ROOM_NUMBER = '01990d9e-8400-7000-8000-000000000401';

test('私人房间：Agent 拿房间号敲门，房间页的提示栈里直接放它进来', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(`${fixturePath}?private&knock`);

  const notifications = page.getByRole('region', { name: 'Notifications' });
  await expect(notifications.getByText('Sol is knocking')).toBeVisible();
  await expect(notifications).toContainText('can read what is sent to it from then on');
  await notifications.getByRole('button', { name: 'Let Sol in' }).click();
  await expect(notifications.getByText('Sol is knocking')).toHaveCount(0);

  // 放进来的和凭口令进来的列在一起，能移出。
  const dialog = await openRoomSettings(page, 'Agent entry');
  await expect(dialog.getByText('No agent is knocking right now.')).toBeVisible();
  await expect(dialog.getByRole('list', { name: 'Agents in this room' })).toContainText('Sol');
  await expect(dialog.getByRole('button', { name: 'Remove Sol' })).toBeVisible();
  await expectAccessibleDialog(page);
  expect(failures).toEqual([]);
});

for (const width of [1_440, 390]) {
  test(`私人房间的接入对话框：话里只有房间号，它敲门后在对话框里放它进来 ${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private`);

    await page
      .getByRole('navigation', { name: 'Room interactions' })
      .getByRole('button', { name: 'Bring an agent', exact: true })
      .click();
    const dialog = page.getByRole('dialog', { name: 'Bring an agent' });
    await dialog.getByRole('radio', { name: /^Network/u }).click();
    await expect(
      dialog.getByText(/a network agent knocks with the room number, and you let it in here/u),
    ).toBeVisible();
    // 打开就能复制，不用先生成什么。房间号不是秘密；口令收在折叠里，话里没有。
    await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeEnabled();
    const message = (await dialog.getByLabel('Message for your agent').textContent()) ?? '';
    expect(message).toContain(ROOM_NUMBER);
    expect(message).toContain('/agents.txt');
    expect(message.toLowerCase()).not.toContain('code');
    await expect(dialog.getByText(/When it knocks, it shows up here/u)).toBeVisible();
    await expect(dialog.getByText('Want the agent in without waiting? Use a code')).toBeVisible();

    await page.evaluate(() => {
      (window as LobbyFixtureWindow).__agentRoomFixtureControls.knockAgent('Atlas');
    });
    const knocks = dialog.getByRole('list', { name: 'Agents knocking' });
    await expect(knocks.getByText('Atlas is knocking')).toBeVisible({ timeout: 10_000 });
    await knocks.getByRole('button', { name: 'Let Atlas in' }).click();
    await expect(dialog.getByText('Atlas is in')).toBeVisible();
    await expect(knocks).toHaveCount(0);
    await expectAccessibleDialog(page);
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
    await page.screenshot({
      animations: 'disabled',
      path: testInfo.outputPath(`agent-knock-${String(width)}.png`),
    });
  });
}
