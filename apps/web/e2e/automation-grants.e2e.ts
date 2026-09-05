import { openRoomSettings } from './support/workspace-navigation';
import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html';

test('自动发言授权从明确影响确认到撤销均保持精确且可见', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  await openRoomSettings(page);
  await page.getByRole('button', { name: 'Automation' }).click();
  const dialog = page.getByRole('dialog', { name: 'Automation grants' });
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole('heading', { name: /Fixture Codex Agent may publish autonomously/u }),
  ).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Create grant' })).toBeDisabled();

  await dialog
    .getByRole('checkbox', {
      name: 'I understand this Agent can send without per-message approval.',
    })
    .check();
  await dialog.getByRole('button', { name: 'Create grant' }).click();

  await expect(dialog.getByText('Active', { exact: true })).toBeVisible();
  await expect(dialog.getByText('0 / 6 this minute')).toBeVisible();
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-29/automation-grant-desktop.png',
  });

  await dialog.getByRole('button', { name: 'Revoke' }).click();
  await expect(dialog.getByText('Revoked', { exact: true })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Revoke' })).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('自动发言授权在窄屏退化为全屏面板且没有横向溢出', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await openRoomSettings(page);
  await page.getByRole('button', { name: 'Automation' }).click();
  await expect(page.getByRole('dialog', { name: 'Automation grants' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-29/automation-grant-mobile.png',
  });
});
