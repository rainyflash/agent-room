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
  // 不在线的 Agent 收着，点开才列出来。
  await expect(page.getByText('Research scout')).toBeHidden();
  await page.getByText('Offline · 3').click();
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

test('7 天以上没用的电脑收着，一键撤销时可以留下几台', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 1_000, width: 1_440 });
  await page.goto(fixturePath);

  const computers = page.locator('article', {
    has: page.getByRole('heading', { name: 'Computers and browsers' }),
  });
  await expect(computers.getByText('Studio workstation')).toBeVisible();
  await expect(computers.getByText('Alpha 70 fresh-device acceptance')).toBeHidden();
  await computers.getByText('Not used for 7 days or more · 2').click();
  await expect(computers.getByText('Expired', { exact: true })).toBeVisible();
  await computers.getByRole('button', { name: 'Revoke all' }).click();

  const dialog = page.getByRole('dialog', { name: 'Revoke computers not used for 7 days or more' });
  await expect(dialog.getByRole('checkbox')).toHaveCount(2);
  await dialog.getByRole('checkbox', { name: /Alpha 70 fresh-device acceptance/u }).uncheck();
  await dialog.getByRole('button', { name: 'Revoke 1 computer' }).click();
  await expect(dialog).toHaveCount(0);

  await expect(computers.getByText('Not used for 7 days or more · 1')).toBeVisible();
  await expect(computers.getByText('Revoked · 2')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('没有最近登录时，一键撤销先请人重新登录', async ({ page }) => {
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(`${fixturePath}?reauth`);

  await page.getByText('Not used for 7 days or more · 2').click();
  await page.getByRole('button', { name: 'Revoke all' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByText('Revoking computers needs a fresh sign-in first.')).toBeVisible();
  await expect(dialog.getByRole('button', { name: /^Revoke \d/u })).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  await dialog.getByRole('button', { name: 'Sign in again' }).click();
  await expect(page.locator('body')).toHaveAttribute('data-sign-in-requested', 'true');
});

for (const width of [1_440, 390]) {
  test(`${String(width)}px 安全里列出屏蔽的 Agent，解除以后拿掉`, async ({ page }) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ height: width < 768 ? 844 : 1_000, width });
    await page.goto(fixturePath);

    const blocks = page.getByRole('region', { name: 'Blocked agents' });
    const recorded = blocks.getByRole('listitem').filter({ hasText: 'Noisy helper #4821' });
    await expect(recorded).toContainText('Blocked');
    // 以前服务器没记上、只在聊天账户忽略名单里的，名字不知道就写 Matrix ID。
    await expect(blocks.getByText('@_agent_0198b60177a17bb8:agent-room.test')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await blocks.screenshot({
      path: `../../artifacts/browser/blocked-agents-${String(width)}.png`,
    });

    await recorded.getByRole('button', { name: 'Unblock' }).click();
    await expect(recorded).toHaveCount(0);
    await expect(blocks.getByRole('listitem')).toHaveCount(1);
    expect(failures).toEqual([]);
  });
}
