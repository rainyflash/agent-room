import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/security-center.html';

test('安全中心在桌面端展示真实状态并完成 SAS 确认', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 1_000, width: 1_440 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { level: 1, name: 'Security' })).toBeVisible();
  await expect(page.getByText('@alice:agent-room.test')).toBeVisible();
  await expect(page.locator('.security-devices__list > li')).toHaveCount(3);
  await expect(page.getByRole('heading', { name: 'Product devices' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Agent instances' })).toBeVisible();
  await expect(page.getByText('Release architect')).toBeVisible();
  await expect(page.getByText('Research scout')).toBeVisible();
  await expectNoHorizontalOverflow(page);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-28/security-desktop.png',
  });

  await page.getByRole('button', { name: 'Stop instance' }).first().click();
  await expect(page.getByText('Stop this Agent instance?')).toBeVisible();
  await expect(
    page.getByText(
      'Release architect will lose its instance lease and dedicated Matrix device session.',
    ),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByText('Stop this Agent instance?')).toHaveCount(0);

  await page.getByRole('button', { name: 'Verify', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Verify a Matrix device' });
  await expect(dialog.locator('.security-sas-emojis > li')).toHaveCount(7);
  await dialog.getByRole('button', { name: 'They match' }).click();
  await expect(dialog.getByText('Device verified')).toBeVisible();
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-27/security-verification.png',
  });
});

test('390px 安全中心无横向溢出且恢复流程可操作', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { name: 'Product devices' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Agent instances' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-28/access-mobile.png',
  });
  await page.getByRole('button', { name: 'Recover this device' }).last().click();
  await expect(page.getByLabel('Passphrase or recovery key')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: false,
    path: '../../artifacts/browser/task-28/security-mobile.png',
  });
});
