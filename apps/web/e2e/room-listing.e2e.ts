import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openRoomSettings } from './support/workspace-navigation';

const roomFixture = '/e2e/fixtures/lobby-scene.html';

async function expectAccessible(page: Page, selector: string): Promise<void> {
  const scan = await new AxeBuilder({ page })
    .include(selector)
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({ id, targets: nodes.map(({ target }) => target) })),
  ).toEqual([]);
}

for (const width of [1440, 390]) {
  test(`房间设置的“公开”：私密设成公开先确认，拿到分享链接，换码，改回私密：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private`);

    const dialog = await openRoomSettings(page, 'Public');
    const visibility = dialog.getByRole('group', { name: 'Who can find this room' });
    await expect(visibility.getByRole('radio', { name: /^Private/u })).toBeChecked();
    await expect(dialog.getByRole('group', { name: 'How people join' })).toHaveCount(0);

    await visibility.getByRole('radio', { name: /^Public/u }).check();
    await dialog.getByRole('radio', { name: /^Ask to join/u }).check();
    await expect(dialog.getByRole('checkbox', { name: /^Manage/u })).toBeDisabled();
    await dialog.getByRole('button', { name: 'Save' }).click();
    await expect(dialog).toContainText('Make this room public?');
    await expect(dialog).toContainText('It stays encrypted');
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page, '.ar-dialog');
    await page.screenshot({
      path: testInfo.outputPath(`room-listing-confirm-${String(width)}.png`),
    });

    await dialog.getByRole('button', { name: 'Make public' }).click();
    await expect(dialog).toContainText('Saved.');
    await expect(dialog.getByLabel('Share link')).toHaveText('https://agentroom.test/r/aB3xYz');

    await dialog.getByRole('button', { name: 'New share code' }).click();
    await dialog.getByRole('button', { name: 'Change it' }).click();
    await expect(dialog.getByLabel('Share link')).toHaveText('https://agentroom.test/r/Q7mPz2');

    await visibility.getByRole('radio', { name: /^Private/u }).check();
    await dialog.getByRole('button', { name: 'Save' }).click();
    await expect(dialog).toContainText('Make this room private?');
    await dialog.getByRole('button', { name: 'Make private' }).click();
    await expect(dialog).toContainText('Saved.');
    await expect(dialog.getByLabel('Share link')).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}

test('房间设置的“公开”：公开着的不加密房间写清楚谁读得到', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${roomFixture}?private&listed`);

  const dialog = await openRoomSettings(page, 'Public');
  await expect(dialog).toContainText('This room is not encrypted');
  await expect(dialog.getByLabel('Share link')).toHaveText('https://agentroom.test/r/aB3xYz');
  await dialog.getByRole('radio', { name: /^Invite only/u }).check();
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog).toContainText('Invite-only rooms have no share link');
  expect(failures).toEqual([]);
});

for (const width of [1440, 390]) {
  test(`公开房间有人申请：齿轮上亮点，“成员”一节里批准、拒绝，设管理员：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private&listed&requests`);

    const gear = page.getByRole('button', { name: 'Room settings', exact: true });
    await expect(gear).toHaveAccessibleDescription('Requests to join: 2');
    await expect(gear.locator('.workspace-header__attention')).toBeVisible();
    await page.locator('.workspace-header').screenshot({
      path: testInfo.outputPath(`room-join-request-dot-${String(width)}.png`),
    });

    const dialog = await openRoomSettings(page, 'Members');
    await expect(dialog).toContainText('Public · Ask to join');
    const requests = dialog.locator('.private-room-join-requests');
    await expect(requests.getByRole('heading', { name: 'Requests to join' })).toBeVisible();
    await expect(requests).toContainText('Can I bring my agent?');
    await requests.scrollIntoViewIfNeeded();
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page, '.ar-dialog');
    await page.screenshot({
      path: testInfo.outputPath(`room-join-requests-${String(width)}.png`),
    });

    await requests.getByRole('button', { name: 'Approve Lin' }).click();
    await expect(requests).not.toContainText('Lin');
    await requests.getByRole('button', { name: 'Reject Mo' }).click();
    await expect(requests).toContainText('No one is waiting.');

    // Grace 原来只能查看和发言：设成管理员以后写着“管理员”，能撤回去。
    await dialog.getByRole('button', { name: 'Make admin' }).click();
    const grace = dialog.locator('.private-room-members > ol > li', { hasText: 'Grace' });
    await expect(grace.locator('.private-room-member__role')).toHaveText('Admin');
    await grace.scrollIntoViewIfNeeded();
    await page.screenshot({
      path: testInfo.outputPath(`room-admin-${String(width)}.png`),
    });
    await grace.getByRole('button', { name: 'Remove admin' }).click();
    await expect(grace.locator('.private-room-member__role')).toHaveCount(0);
    expect(failures).toEqual([]);

    await dialog.getByRole('button', { name: 'Close room settings' }).click();
    await expect(gear).not.toHaveAccessibleDescription('Requests to join: 2');
    await expect(gear.locator('.workspace-header__attention')).toHaveCount(0);
  });
}
