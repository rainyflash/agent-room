import { openRoomSettings } from './support/workspace-navigation';
import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html';

test('旧的有效登录直接创建并撤销授权，全程不跳转身份验证', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(`${fixturePath}?olderSession=1`);

  const dialog = await openRoomSettings(page, 'Automation');
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole('heading', { name: /Fixture Codex Agent may speak on its own/u }),
  ).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Allow', exact: true })).toBeDisabled();
  await expect(dialog.getByRole('button', { name: 'Sign in again' })).toHaveCount(0);

  await dialog
    .getByRole('checkbox', {
      name: 'I understand it will speak without asking me each time.',
    })
    .check();
  await dialog.getByRole('button', { name: 'Allow', exact: true }).click();

  await expect(dialog.getByText('Active', { exact: true })).toBeVisible();
  await expect(dialog.getByText('0 / 6 this minute')).toBeVisible();
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-29/automation-grant-desktop.png',
  });

  await dialog.getByRole('button', { name: 'Stop' }).click();
  await expect(dialog.getByText('Stopped', { exact: true })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Stop' })).toHaveCount(0);
  await expect(page).toHaveURL(/olderSession=1/u);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('房间设置的自动发言在窄屏是贴底面板且没有横向溢出', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(`${fixturePath}?olderSession=1`);

  const dialog = await openRoomSettings(page, 'Automation');
  await expect(dialog.getByRole('button', { name: 'Allow', exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-29/automation-grant-mobile.png',
  });
});
