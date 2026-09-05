import { openRoomSettings } from './support/workspace-navigation';
import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html';

test('私人房间三步创建只陈述真实安全边界', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  await openRoomSettings(page);
  await page.getByRole('button', { name: 'Private rooms' }).click();
  const dialog = page.getByRole('dialog', { name: 'Private rooms' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('No private room is connected yet');

  await dialog.getByRole('button', { name: 'Create room' }).click();
  await dialog.getByLabel('Room name').fill('Architecture review');
  await dialog.getByLabel('Purpose').fill('Coordinate a bounded design review.');
  await dialog.getByRole('button', { name: 'Continue' }).click();

  await expect(dialog.getByText('Invite known principals')).toBeVisible();
  await dialog.getByLabel('Automate').check();
  await expect(dialog.getByLabel('Speak')).toBeChecked();
  await dialog.getByRole('button', { name: 'Continue' }).click();

  await expect(dialog.getByText('Invite-only Matrix boundary')).toBeVisible();
  await expect(dialog.getByText('TASK 27', { exact: true })).toBeVisible();
  await expect(dialog).toContainText('does not label transport access as end-to-end encryption');
  await dialog.getByRole('button', { name: 'Provision and join' }).click();
  await expect(dialog).toContainText('private_room.fixture_unavailable');
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-25/private-room-security-truth.png',
  });
});

test('私人房间 Sheet 在手机上不产生横向溢出', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await openRoomSettings(page);
  await page.getByRole('button', { name: 'Private rooms' }).click();
  await expect(page.getByRole('dialog', { name: 'Private rooms' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});
