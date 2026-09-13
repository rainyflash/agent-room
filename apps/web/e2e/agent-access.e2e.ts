import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`一键接入 Agent：复制专属指令并观察到达 ${String(width)}px`, async ({
    page,
    context,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/e2e/fixtures/onboarding.html');
    await page.getByRole('button', { name: /Local agents/u }).click();
    await expect(page.getByText(/No agent task has opened/u)).toBeVisible();
    await page.getByRole('complementary').getByRole('button', { name: 'Bring an agent' }).click();
    const dialog = page.getByRole('dialog', { name: 'Bring an agent into the room' });
    await expect(dialog.getByText(/no MCP setup is needed/u)).toBeVisible();
    await expect(dialog.getByRole('button', { name: /Configure Codex/u })).toHaveCount(0);
    await expect(
      dialog.getByText('Ready. Copy the instructions above and send them to your agent.'),
    ).toBeVisible();
    await dialog.getByRole('button', { name: 'Copy connection instructions' }).click();
    await expect(dialog.getByText('Copied. Paste it to your agent.')).toBeVisible();
    const clipboard = await page.evaluate(() => navigator.clipboard.readText());
    const invitationToken = /join --invite ([A-Za-z0-9_-]+)/u.exec(clipboard)?.[1];
    expect(invitationToken).toBeDefined();
    const invitation: unknown = JSON.parse(
      Buffer.from(invitationToken ?? '', 'base64url').toString('utf8'),
    );
    expect(invitation).toMatchObject({ displayName: 'Fixture operator’s agent', version: 1 });
    const profileId = /--profile ([0-9a-f-]{36})/u.exec(clipboard)?.[1];
    expect(profileId).toBeDefined();
    expect(clipboard).toContain('untrusted input');
    await page.screenshot({ path: testInfo.outputPath(`agent-invite-${String(width)}.png`) });
    await expectNoHorizontalOverflow(page);
    await dialog.getByRole('button', { name: 'Close' }).click();
    await expect(dialog).toHaveCount(0);

    // 新邀请默认独立人物；只有明确选择保存的人物后才确认该连接到达。
    await page.goto('/e2e/fixtures/onboarding.html?host=ready');
    await page.getByRole('button', { name: /Local agents/u }).click();
    await expect(page.getByText('Scout', { exact: true })).toBeVisible();
    await expect(page.getByText('Messages fetched · No confirmed send yet')).toBeVisible();
    await page.getByRole('complementary').getByRole('button', { name: 'Bring an agent' }).click();
    await expect(dialog.getByText('“Scout” is in the room')).toHaveCount(0);
    await dialog.getByLabel('Saved characters').selectOption(profileId ?? '');
    await expect(dialog.getByText('“Scout” is in the room')).toBeVisible();
    await dialog.getByRole('button', { name: 'Copy connection instructions' }).click();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(clipboard);
    await dialog.getByRole('button', { name: 'Done' }).click();
    await expect(dialog).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath(`agent-access-${String(width)}.png`) });
    expect(failures).toEqual([]);
  });
}

test('连接检查失败提供诊断而非伪造空清单', async ({ page }) => {
  await page.goto('/e2e/fixtures/onboarding.html?host=failed');
  await page.getByRole('button', { name: /Local agents/u }).click();
  await expect(page.getByText('fixture.external_action_unavailable')).toBeVisible();
  await expect(page.getByText(/No agent task has opened/u)).toHaveCount(0);
  await page.getByRole('complementary').getByRole('button', { name: 'Bring an agent' }).click();
  await expect(
    page.getByRole('dialog').getByText(/Cannot check task connections right now/u),
  ).toBeVisible();
});

test('已授权空设备直接接入；重连恢复无需重新登录', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/onboarding.html?bridge=reconnecting&configured=1');
  await page.getByRole('button', { name: /Local agents/u }).click();
  await page.getByRole('complementary').getByRole('button', { name: 'Bring an agent' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByText(/Reconnecting automatically/u)).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Copy connection instructions' })).toBeDisabled();
  await dialog.getByRole('button', { name: 'Retry connection' }).click();
  await expect(dialog.getByRole('button', { name: 'Copy connection instructions' })).toBeEnabled();
  await expect(dialog.getByText(/Reconnecting automatically/u)).toHaveCount(0);
  await expect(dialog.getByText(/Finish authorization/u)).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('MCP 配置失败不会阻止默认 CLI 接入，兼容方式保留诊断', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/onboarding.html?bridge=authorized&setup=failed');
  await page.getByRole('button', { name: /Local agents/u }).click();
  await page.getByRole('complementary').getByRole('button', { name: 'Bring an agent' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog.getByRole('button', { name: 'Copy connection instructions' })).toBeEnabled();
  await expect(dialog.getByText(/cannot read your current settings/u)).toHaveCount(0);
  await dialog.getByText('Other connection options', { exact: true }).click();
  await dialog.getByRole('radio', { name: 'MCP compatibility', exact: true }).click();
  await expect(dialog.getByText(/cannot read your current settings/u)).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Copy connection instructions' })).toBeDisabled();
  await expect(
    dialog.getByText('Waiting for the agent to run its connection instructions…'),
  ).toHaveCount(0);
  await expect(dialog.getByText(/Finish connecting this computer above/u)).toBeVisible();
  await dialog.getByRole('radio', { name: 'CLI (default)', exact: true }).click();
  await expect(dialog.getByRole('button', { name: 'Copy connection instructions' })).toBeEnabled();
  expect(failures).toEqual([]);
});
