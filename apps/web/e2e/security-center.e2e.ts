import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/security-center.html';

test('390px 桌面 Agent 恢复设置可操作且密钥确认后消失', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(`${fixturePath}?agentRecovery`);
  const panel = page.getByRole('region', { name: 'Agent recovery on this computer' });
  await panel.getByRole('button', { name: 'Set up recovery key' }).click();
  await panel.getByLabel('Recovery passphrase', { exact: true }).fill('long fixture passphrase');
  await panel.getByLabel('Confirm passphrase', { exact: true }).fill('long fixture passphrase');
  await panel.getByRole('button', { name: 'Create recovery' }).click();
  await expect(panel.getByText('EsTc test-only recovery-key save-outside-this-app')).toBeVisible();
  await expect(panel.getByRole('combobox')).toBeDisabled();
  await expectNoHorizontalOverflow(page);
  await panel.screenshot({ path: '../../artifacts/browser/agent-recovery-mobile.png' });
  await panel.getByRole('button', { name: 'I saved the recovery key' }).click();
  await expect(panel.getByText('EsTc test-only recovery-key save-outside-this-app')).toHaveCount(0);
  await expect(panel.getByText('Identity and recovery are ready')).toBeVisible();
  expect(failures).toEqual([]);
});

test('安全中心在桌面端展示真实状态并完成 SAS 确认', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 1_000, width: 1_440 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { level: 1, name: 'Settings' })).toBeVisible();
  await expect(page.getByRole('heading', { level: 2, name: 'Security' })).toBeVisible();
  // 你的设备：名字、是不是这台、签没签名；Matrix ID 和指纹收在详情里。
  await expect(page.locator('.security-devices__list > li')).toHaveCount(3);
  await expect(page.getByText('@alice:agent-room.test')).toBeHidden();
  await expect(page.getByText(/Crypto engine/u)).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Computers and browsers' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Agents', exact: true })).toBeVisible();
  await expect(page.getByText('Release architect')).toBeVisible();
  await expect(page.getByText('Research scout')).toBeVisible();
  await expectNoHorizontalOverflow(page);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-28/security-desktop.png',
  });

  await page.getByRole('button', { name: 'Disconnect' }).first().click();
  await expect(page.getByText('Disconnect this agent?')).toBeVisible();
  await expect(
    page.getByText('Release architect will be disconnected. It can join again later.'),
  ).toBeVisible();
  await page.getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByText('Disconnect this agent?')).toHaveCount(0);

  // 这台设备已由你签名；没签名的别的设备可以在列表里核对。
  await expect(page.getByText('This device is signed by you')).toBeVisible();
  await page.getByRole('button', { name: 'Verify', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Verify a device' });
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

  await expect(page.getByRole('heading', { name: 'Computers and browsers' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Agents', exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-28/access-mobile.png',
  });
  await page
    .getByRole('region', { name: 'Recovery key', exact: true })
    .getByRole('button', { name: 'Enter recovery key' })
    .click();
  await expect(page.getByLabel('Passphrase or recovery key')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: false,
    path: '../../artifacts/browser/task-28/security-mobile.png',
  });
});
