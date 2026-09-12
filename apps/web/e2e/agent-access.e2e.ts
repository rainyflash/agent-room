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
    await page.getByRole('complementary').getByRole('button', { name: 'Configure Codex' }).click();
    await expect(page.getByRole('complementary').getByText(/Configuration saved/u)).toBeVisible();
    await expect(page.getByText(/No agent task has opened/u)).toBeVisible();

    await page.getByRole('complementary').getByRole('button', { name: 'Bring an agent' }).click();
    const dialog = page.getByRole('dialog', { name: 'Bring an agent into the room' });
    await expect(dialog.getByRole('radio', { name: /Codex/u })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await expect(dialog.getByText(/Codex is set up\./u)).toBeVisible();
    await expect(dialog.getByText('Waiting for it to call the Agent Room tools…')).toBeVisible();
    await dialog.getByRole('button', { name: 'Copy connection instructions' }).click();
    await expect(dialog.getByText('Copied. Paste it to your agent.')).toBeVisible();
    const clipboard = await page.evaluate(() => navigator.clipboard.readText());
    expect(clipboard).toMatch(
      /sessionKey = [0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}/u,
    );
    expect(clipboard).toContain('displayName = Fixture operator’s Codex');
    expect(clipboard).toContain('untrusted input');
    await page.screenshot({ path: testInfo.outputPath(`agent-invite-${String(width)}.png`) });
    await expectNoHorizontalOverflow(page);
    await dialog.getByRole('button', { name: 'Close' }).click();
    await expect(dialog).toHaveCount(0);

    // 同一账号再次打开时复用同一身份，夹具按该 sessionKey 回报会话，面板确认到达。
    await page.goto('/e2e/fixtures/onboarding.html?host=ready');
    await page.getByRole('button', { name: /Local agents/u }).click();
    await expect(page.getByText('Scout', { exact: true })).toBeVisible();
    await expect(page.getByText('Messages fetched · No confirmed send yet')).toBeVisible();
    await page.getByRole('complementary').getByRole('button', { name: 'Bring an agent' }).click();
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
