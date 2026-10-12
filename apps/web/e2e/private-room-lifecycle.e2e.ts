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

for (const width of [1_440, 390]) {
  test(`网络 Agent 建的私人房间：房间顶上、设置里都说服务器能读到 ${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private&origin=network`);
    const label = page.locator('.workspace-header__identity > span');
    await expect(label).toHaveText('Private room · The server can read it');
    await expect(label).toHaveAttribute(
      'title',
      'A network agent made this room. The server sends and receives for it, so the server can read everything said here.',
    );

    const dialog = await openRoomSettings(page);
    await expect(dialog.getByText('The server can read this room')).toBeVisible();
    await expect(dialog.getByText('Visitor (an agent)')).toBeVisible();
    await expect(dialog.getByText('Visitor (network agent)')).toBeVisible();
    await expect(
      dialog.getByText(
        'End-to-end encrypted. A network agent made this room, so the server can read what is said here.',
      ),
    ).toBeVisible();
    // 网络 Agent 邀请来的人只能查看和发言：不能改名，也没有“Agent 进门”一节。
    await expect(dialog.getByRole('textbox', { name: 'Room name' })).toHaveCount(0);
    await expect(dialog.getByRole('radio', { name: 'Agent entry' })).toHaveCount(0);
    await expectAccessibleDialog(page);
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
    await page.screenshot({
      animations: 'disabled',
      path: testInfo.outputPath(`server-readable-${String(width)}.png`),
    });
  });
}

test('本机 Agent 建的私人房间：设置里写上谁建的，别的照旧', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${fixturePath}?private&origin=local`);
  await expect(page.locator('.workspace-header__identity > span')).toHaveText('Private room');

  const dialog = await openRoomSettings(page);
  await expect(dialog.getByText('Scout (an agent)')).toBeVisible();
  await expect(dialog.getByText('End-to-end encrypted', { exact: true })).toBeVisible();
  await expect(dialog.getByText('The server can read this room')).toHaveCount(0);
  // 房主还是你：能改名，也有“Agent 进门”一节。
  await expect(dialog.getByRole('textbox', { name: 'Room name' })).toBeVisible();
  await expect(dialog.getByRole('radio', { name: 'Agent entry' })).toBeVisible();
  expect(failures).toEqual([]);
});

for (const width of [1_440, 390]) {
  test(`归档了的私人房间：之前的消息照样看，输入框关着并说清为什么，也不能再接入 Agent ${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private&origin=network&archived&view=conversation&history=1`);
    // 归档比“服务器能读到”先说。
    const label = page.locator('.workspace-header__identity > span');
    await expect(label).toHaveText('Private room · Archived');
    await expect(label).toHaveAttribute(
      'title',
      'This room is archived. You can still read earlier messages, but nobody can send new ones.',
    );
    const room = page.locator('.workspace-room-content');
    await expect(room.getByRole('log').locator('article').first()).toBeVisible();
    await expect(page.locator('.room-panel__read-only')).toHaveText(
      'This room is archived. You can still read earlier messages, but nobody can send new ones.',
    );
    await expect(room.getByRole('textbox', { name: 'Message', exact: true })).toBeDisabled();
    await expect(room.getByRole('button', { name: 'Send', exact: true })).toBeDisabled();
    await expect(
      page
        .getByRole('navigation', { name: 'Room interactions' })
        .getByRole('button', { name: 'Bring an agent', exact: true }),
    ).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
    await page.screenshot({
      animations: 'disabled',
      path: testInfo.outputPath(`archived-room-${String(width)}.png`),
    });
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

/** 打开房间页上的“接入 Agent”，选网络接入。 */
async function openNetworkInvite(page: Page) {
  await page
    .getByRole('navigation', { name: 'Room interactions' })
    .getByRole('button', { name: 'Bring an agent', exact: true })
    .click();
  const dialog = page.getByRole('dialog', { name: 'Bring an agent' });
  await dialog.getByRole('radio', { name: /^Network/u }).click();
  return dialog;
}

const JOIN_CODE =
  /with the code [0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}/u;

for (const width of [1_440, 390]) {
  test(`私人房间的接入对话框：一键生成口令并复制，话里带着敲门用的房间号，它敲门后在对话框里放它进来 ${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private`);

    const dialog = await openNetworkInvite(page);
    await expect(
      dialog.getByText(/a network agent comes straight in with its code/u),
    ).toBeVisible();
    // 还没有口令：一个按钮生成并复制。不想用口令的，折叠里有只带房间号的那段话。
    await expect(dialog.getByText('Rather not use a code? Let it knock')).toBeVisible();
    await dialog.getByRole('button', { name: 'Create code and copy' }).click();
    await expect(dialog.getByText(/Anyone who gets the code can come in/u)).toBeVisible();
    const message = dialog.getByLabel('Message for your agent');
    await expect(message).toContainText(JOIN_CODE);
    await expect(message).toContainText(`knock with the room number ${ROOM_NUMBER}`);
    await expect(message).toContainText('/agents.txt');
    await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeEnabled();
    await expect(
      dialog.getByText(/If your agent knocks with the room number, it shows up here/u),
    ).toBeVisible();

    // 口令传不进去的 Agent 照话里的房间号敲门。
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

test('私人房间已经有口令：接入对话框打开就带着它，房间设置里也看得到、能复制', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(`${fixturePath}?private&code`);

  const dialog = await openNetworkInvite(page);
  await expect(dialog.getByLabel('Message for your agent')).toContainText(
    'with the code K7P3-Q9XW-2DMA',
  );
  await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeEnabled();
  await expect(dialog.getByRole('button', { name: /code and copy/u })).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Close', exact: true }).click();

  const settings = await openRoomSettings(page, 'Agent entry');
  await expect(settings.getByText('K7P3-Q9XW-2DMA', { exact: true })).toBeVisible();
  await expect(settings.getByRole('button', { name: 'Copy message for the agent' })).toBeVisible();
  await expectAccessibleDialog(page);
  expect(failures).toEqual([]);
});
