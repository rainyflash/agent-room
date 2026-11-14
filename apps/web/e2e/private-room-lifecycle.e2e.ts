import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Locator, type Page } from '@playwright/test';

import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
import { openRoomSettings, roomAction } from './support/workspace-navigation';

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
/** 房间设置“成员”里某个人的那一行。 */
function memberRow(dialog: Locator, name: string): Locator {
  return dialog.locator('.room-members > li', { hasText: name });
}
/** 房间设置“成员”一节最下面的“身份变动”：收着的那一行、每条说的事、每条下面谁动的手。 */
function roleChanges(dialog: Locator) {
  const section = dialog.locator('.room-role-changes');
  return {
    bylines: section.locator('.room-role-changes__list > li > small'),
    section,
    sentences: section.locator('.room-role-changes__list > li > p'),
    summary: section.locator('summary'),
  };
}
/** 顶栏上房间名下面那一小句：这是哪种房间、谁读得到。 */
function roomKind(page: Page) {
  return page.locator('.room-desk__meta > span').first();
}
/** 顶栏上的“接入 Agent”。 */
function bringAgent(page: Page) {
  return page
    .locator('.room-desk__top')
    .getByRole('button', { name: 'Bring an agent', exact: true });
}
const first = '0198b601-77a2-7f41-b4f4-940f291951b8';
const second = '0198b601-77a2-7f41-b4f4-940f291951b9';

test('新建房间一屏：保留多久、现在就邀请谁放在更多选项里', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(`${fixturePath}?features=1&failJoin=1`);
  await expect(page.getByRole('heading', { name: 'Your rooms' })).toBeVisible();
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
  await expect(dialog).toContainText('fixture.unavailable');
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
  await page.goto(`${fixturePath}?features=1`);
  await expect(page.getByRole('heading', { name: 'Your rooms' })).toBeVisible();
  await page.getByRole('button', { name: 'New room', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'New room' });
  await expect(dialog).toBeVisible();
  await dialog.getByText('More options').click();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

for (const width of [1_440, 390]) {
  test(`私人房间的房间设置：六节在一个对话框里 ${String(width)}`, async ({ page }) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private`);
    await expect(roomKind(page)).toHaveText('Private room');

    const dialog = await openRoomSettings(page);
    const sections = dialog.getByRole('radiogroup', { name: 'Room settings sections' });
    await expect(sections.getByRole('radio')).toHaveText([
      'Members',
      'Public',
      'Notice',
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
  test(`成员写昵称、能解除封禁、转移房主前先确认 ${String(width)}`, async ({ page }) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private`);

    const dialog = await openRoomSettings(page);
    const members = dialog.locator('.private-room-members');
    const grace = memberRow(dialog, 'Grace');
    const mallory = memberRow(dialog, 'Mallory');
    await expect(grace).toBeVisible();
    await expect(mallory).toContainText('Banned');
    // 账户 ID、解除封禁、转让房主都收在每一行的“更多”里。
    await expect(members.getByText('0198b601-77a1-7bb8-83eb-a8fe68c97e43')).toHaveCount(0);
    await expect(members.getByRole('button', { name: 'Unban' })).toHaveCount(0);

    await mallory.getByRole('button', { name: 'More about Mallory' }).click();
    await mallory.getByRole('button', { name: 'Unban' }).click();
    await expect(members.getByRole('button', { name: 'Unban' })).toHaveCount(0);
    await expect(mallory).toContainText('Removed');

    await grace.getByRole('button', { name: 'More about Grace' }).click();
    await expect(grace.getByText('0198b601-77a1-7bb8-83eb-a8fe68c97e43')).toBeVisible();
    await grace.getByRole('button', { name: 'Transfer ownership' }).click();
    const confirm = members.getByRole('group', { name: 'Make Grace the owner?' });
    await expect(confirm).toBeVisible();
    await expect(confirm.getByRole('checkbox', { name: /^Manage/u })).toBeChecked();
    await expectNoHorizontalOverflow(page);
    await confirm.getByRole('button', { name: 'Cancel' }).click();
    await expect(confirm).toHaveCount(0);
    await expectAccessibleDialog(page);
    expect(failures).toEqual([]);
  });
}

for (const width of [1_440, 390]) {
  test(`成员的身份：四种身份的说明，身份选一下，更多里两个开关 ${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private&owners`);

    const dialog = await openRoomSettings(page, 'Members');
    const legend = dialog.locator('.room-roles');
    for (const role of ['Owner', 'Admin', 'Member', 'View only']) {
      await expect(legend.getByText(role, { exact: true })).toBeVisible();
    }

    // 你是房主：自己那一行只写字，没有选择框。
    const ada = memberRow(dialog, 'Ada');
    await expect(ada).toContainText('you');
    await expect(ada.locator('.room-member__role')).toHaveText('Owner');
    await expect(ada.getByRole('combobox')).toHaveCount(0);

    const grace = memberRow(dialog, 'Grace');
    const role = grace.getByRole('combobox', { name: 'Role of Grace' });
    await expect(role).toHaveValue('member');
    await expect(role.getByRole('option')).toHaveText(['Admin', 'Member', 'View only']);
    // 每个人带了几个 Agent：窄了这一格收起来。
    if (width === 390) await expect(grace.locator('.room-member__agents')).toBeHidden();
    else await expect(grace.locator('.room-member__agents')).toHaveText('6 agents');

    await grace.getByRole('button', { name: 'More about Grace' }).click();
    const agentsSpeak = grace.getByRole('switch', { name: 'Agents they bring can speak' });
    const canInvite = grace.getByRole('switch', { name: 'Can invite people' });
    await expect(agentsSpeak).not.toBeChecked();
    await expect(canInvite).not.toBeChecked();

    // 只能看：两个开关都关着、动不了。改回成员，带来的 Agent 按默认能说话。
    await role.selectOption('viewer');
    await expect(role).toHaveValue('viewer');
    await expect(agentsSpeak).toBeDisabled();
    await expect(canInvite).toBeDisabled();
    await role.selectOption('member');
    await expect(agentsSpeak).toBeChecked();
    await expect(canInvite).not.toBeChecked();
    await canInvite.check();
    await expect(canInvite).toBeChecked();
    await agentsSpeak.uncheck();
    await expect(agentsSpeak).not.toBeChecked();

    // 设成管理员：能邀请人跟着开，带来的 Agent 能不能说话照旧。
    await role.selectOption('admin');
    await expect(role).toHaveValue('admin');
    await expect(canInvite).toBeChecked();
    await expect(agentsSpeak).not.toBeChecked();

    await grace.scrollIntoViewIfNeeded();
    await expectNoHorizontalOverflow(page);
    await expectAccessibleDialog(page);
    await page.screenshot({ path: testInfo.outputPath(`member-roles-${String(width)}.png`) });
    expect(failures).toEqual([]);
  });

  test(`设、撤管理员和转让房主要最近登录过，不是就先请人重新登录 ${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    // olderSession：登录已经超过 5 分钟。
    await page.goto(`${fixturePath}?private&olderSession`);

    const dialog = await openRoomSettings(page, 'Members');
    const grace = memberRow(dialog, 'Grace');
    const role = grace.getByRole('combobox', { name: 'Role of Grace' });

    // 成员和只能看之间换不用重新登录。
    await role.selectOption('viewer');
    await expect(role).toHaveValue('viewer');
    await role.selectOption('member');
    await expect(role).toHaveValue('member');

    await role.selectOption('admin');
    const question = grace.getByRole('group', { name: /^This needs a fresh sign-in/u });
    await expect(question).toBeVisible();
    await expect(question.getByRole('button', { name: 'Sign in again' })).toBeVisible();
    await expect(role).toHaveValue('member');
    await question.scrollIntoViewIfNeeded();
    await expectNoHorizontalOverflow(page);
    await expectAccessibleDialog(page);
    await page.screenshot({ path: testInfo.outputPath(`member-sign-in-${String(width)}.png`) });
    await question.getByRole('button', { name: 'Cancel' }).click();
    await expect(question).toHaveCount(0);

    // 转让房主以前在这时候不给按钮；现在给，点了先请人重新登录。
    await grace.getByRole('button', { name: 'More about Grace' }).click();
    await grace.getByRole('button', { name: 'Transfer ownership' }).click();
    await expect(question).toBeVisible();
    await expect(dialog.getByRole('group', { name: 'Make Grace the owner?' })).toHaveCount(0);
    expect(failures).toEqual([]);
  });

  test(`管理员动不了房主，能把自己降成成员，先确认一次 ${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private&asAdmin`);

    const dialog = await openRoomSettings(page, 'Members');
    const grace = memberRow(dialog, 'Grace');
    await expect(grace.locator('.room-member__role')).toHaveText('Owner');
    await expect(grace.getByRole('combobox')).toHaveCount(0);

    const ada = memberRow(dialog, 'Ada');
    const role = ada.getByRole('combobox', { name: 'Role of Ada' });
    await expect(role).toHaveValue('admin');
    await expect(role.getByRole('option')).toHaveText(['Admin', 'Member']);
    await role.selectOption('member');
    const question = ada.getByRole('group', { name: /^Stop being an admin\?/u });
    await expect(question).toBeVisible();
    await expect(role).toHaveValue('admin');
    await question.scrollIntoViewIfNeeded();
    await expectNoHorizontalOverflow(page);
    await expectAccessibleDialog(page);
    await page.screenshot({ path: testInfo.outputPath(`member-step-down-${String(width)}.png`) });
    await question.getByRole('button', { name: 'Step down' }).click();
    // 不当了以后自己那一行只写字，别人的身份也换不了了。
    await expect(ada.locator('.room-member__role')).toHaveText('Member');
    await expect(dialog.locator('.room-members').getByRole('combobox')).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}

for (const width of [1_440, 390]) {
  test(`身份变动：收在成员一节最下面，点开是谁把谁从什么改成了什么，改了身份上面就多一条 ${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private`);

    const dialog = await openRoomSettings(page, 'Members');
    const changes = roleChanges(dialog);
    // 平时收着，没点开就不去读。
    await expect(changes.summary).toHaveText('Role changes');
    await expect(changes.sentences).toHaveCount(0);
    await changes.summary.click();
    await expect(changes.sentences).toHaveText([
      'Grace went from Admin to Member',
      'Mallory (Admin) was banned',
      'Grace went from Member to Admin',
    ]);
    await expect(changes.bylines).toHaveText([/^By Ada · /u, /^By Ada · /u, /^By Ada · /u]);

    // 把 Grace 设成管理员，再让她带来的 Agent 能说话：开着的记录跟着多两条，新的在最上面。
    const grace = memberRow(dialog, 'Grace');
    await grace.getByRole('combobox', { name: 'Role of Grace' }).selectOption('admin');
    await expect(changes.sentences).toHaveCount(4);
    await expect(changes.sentences.first()).toHaveText('Grace went from Member to Admin');
    await grace.getByRole('button', { name: 'More about Grace' }).click();
    await grace.getByRole('switch', { name: 'Agents they bring can speak' }).check();
    await expect(changes.sentences).toHaveCount(5);
    await expect(changes.sentences.first()).toHaveText('Agents Grace brings can now speak');
    // 动手的是你。
    await expect(changes.bylines.first()).toHaveText(/^By Ada · /u);

    await changes.section.scrollIntoViewIfNeeded();
    await expectNoHorizontalOverflow(page);
    await expectAccessibleDialog(page);
    await page.screenshot({ path: testInfo.outputPath(`role-changes-${String(width)}.png`) });
    expect(failures).toEqual([]);
  });

  test(`身份变动：管理员也看得到，自己不当管理员了就看不到了 ${String(width)}`, async ({
    page,
  }) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    await page.goto(`${fixturePath}?private&asAdmin`);

    const dialog = await openRoomSettings(page, 'Members');
    const changes = roleChanges(dialog);
    await changes.summary.click();
    await expect(changes.sentences).toHaveText([
      'Mallory (Admin) was banned',
      'Ada went from Member to Admin',
    ]);
    await expect(changes.bylines).toHaveText([/^By Grace · /u, /^By Grace · /u]);

    const ada = memberRow(dialog, 'Ada');
    await ada.getByRole('combobox', { name: 'Role of Ada' }).selectOption('member');
    await ada
      .getByRole('group', { name: /^Stop being an admin\?/u })
      .getByRole('button', { name: 'Step down' })
      .click();
    await expect(ada.locator('.room-member__role')).toHaveText('Member');
    await expect(changes.section).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}

test('身份变动：不是房主也不是管理员的成员看不到', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  // 网络 Agent 建的房间里你只是它请来的成员。
  await page.goto(`${fixturePath}?private&origin=network`);

  const dialog = await openRoomSettings(page, 'Members');
  await expect(memberRow(dialog, 'Ada').locator('.room-member__role')).toHaveText('Member');
  await expect(roleChanges(dialog).section).toHaveCount(0);
  await expect(dialog.getByText('Role changes')).toHaveCount(0);
  expect(failures).toEqual([]);
});

for (const width of [1_440, 390]) {
  test(`归档房间不用最近登录过，先在原处确认，取消就不归档 ${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: 900, width });
    // olderSession：登录已经超过 5 分钟，以前归档按钮是灰的。
    await page.goto(`${fixturePath}?private&olderSession`);

    const dialog = await openRoomSettings(page, 'Members');
    const archive = dialog.getByRole('button', { name: 'Archive room' });
    await expect(archive).toBeEnabled();
    await archive.click();
    const confirm = dialog.getByRole('group', { name: /^Archive “.+”\?$/u });
    await expect(confirm).toContainText('Archiving can’t be undone.');
    await expect(confirm.getByRole('button', { name: 'Cancel' })).toBeFocused();
    await confirm.scrollIntoViewIfNeeded();
    await expectNoHorizontalOverflow(page);
    await expectAccessibleDialog(page);
    await page.screenshot({ path: testInfo.outputPath(`settings-archive-${String(width)}.png`) });
    await confirm.getByRole('button', { name: 'Cancel' }).click();
    await expect(confirm).toHaveCount(0);
    await expect(archive).toBeFocused();
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
    const label = roomKind(page);
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
    // 网络 Agent 邀请来的人不能管理：不能改名，也没有“Agent 进门”一节。
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
  await expect(roomKind(page)).toHaveText('Private room');

  const dialog = await openRoomSettings(page);
  await expect(dialog.getByText('Scout (an agent)')).toBeVisible();
  await expect(dialog.getByText('End-to-end encrypted', { exact: true })).toBeVisible();
  await expect(dialog.getByText('The server can read this room')).toHaveCount(0);
  // 房主还是你：能改名，也有“Agent 进门”一节。
  await expect(dialog.getByRole('textbox', { name: 'Room name' })).toBeVisible();
  await expect(dialog.getByRole('radio', { name: 'Agent entry' })).toBeVisible();
  expect(failures).toEqual([]);
});

test('本机 Agent 刚替你建的房间：Matrix 上你还只是受邀，点进来就替你加入，不叫你回列表', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.goto(`${fixturePath}?private&origin=local&matrix-invite&view=conversation`);
  await expect(roomKind(page)).toHaveText('Private room');
  const room = page.locator('.workspace-room-content');
  await expect(room.getByRole('textbox', { name: 'Message', exact: true })).toBeEnabled();
  await expect(page.getByText('You are not in this room yet')).toHaveCount(0);
  const joins = await page.evaluate(() =>
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.matrixJoins(),
  );
  expect(joins).toHaveLength(1);
  expect(failures).toEqual([]);
});

test('替你加入没成：说打不开，点“重试”再加入一次就进去了', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${fixturePath}?private&origin=local&matrix-invite=fail`);
  await expect(page.getByText('You are not in this room yet')).toBeVisible();
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(roomKind(page)).toHaveText('Private room');
  const joins = await page.evaluate(() =>
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.matrixJoins(),
  );
  expect(joins).toHaveLength(2);
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
    const label = roomKind(page);
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
    // 房间设置还在（手机上在“更多”里），“接入 Agent”顶栏上、“更多”里都没有。
    await expect(await roomAction(page, 'Room settings')).toBeVisible();
    await expect(bringAgent(page)).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Bring an agent', exact: true })).toHaveCount(0);
    if (width === 390) await page.keyboard.press('Escape');
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

/** 打开房间页上的“接入 Agent”（手机上在“更多”里），选网络接入。 */
async function openNetworkInvite(page: Page) {
  await (await roomAction(page, 'Bring an agent')).click();
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
