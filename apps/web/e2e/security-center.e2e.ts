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

test('安全中心在桌面端说这台设备已就绪，没有恢复密钥和核对', async ({ page }) => {
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

  // 设备登录后自动签好（ADR 0011）：只说已就绪，没有恢复密钥、没有核对按钮。
  await expect(page.getByText('This device is ready')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Verify', exact: true })).toHaveCount(0);
  await expect(page.getByText(/recovery key/iu)).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('390px 安全中心无横向溢出，自动签名出错时点重试就好', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(`${fixturePath}?signing=failed`);

  await expect(page.getByRole('heading', { name: 'Computers and browsers' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Agents', exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-28/access-mobile.png',
  });
  await expect(page.getByText('This device isn’t ready yet')).toBeVisible();
  await page.getByRole('button', { name: 'Try again' }).click();
  await expect(page.getByText('Getting this device ready')).toBeVisible();
  await expect(page.getByText('This device is ready')).toBeVisible();
  await expect(
    page.locator('.security-devices__list > li.is-current').getByText('Signed by you'),
  ).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: false,
    path: '../../artifacts/browser/task-28/security-mobile.png',
  });
});
