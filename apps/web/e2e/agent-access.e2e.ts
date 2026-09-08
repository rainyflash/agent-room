import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`MCP 接入证据与配置反馈 ${String(width)}px`, async ({ page, context }, testInfo) => {
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
    await page.getByRole('button', { name: 'Copy connection instructions' }).click();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toContain(
      'unique to this task',
    );
    await page.goto('/e2e/fixtures/onboarding.html?host=ready');
    await page.getByRole('button', { name: /Local agents/u }).click();
    await expect(page.getByText('Scout', { exact: true })).toBeVisible();
    await expect(page.getByText('Messages fetched · No confirmed send yet')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath(`agent-access-${String(width)}.png`) });
    expect(failures).toEqual([]);
  });
}

test('连接检查失败提供诊断而非伪造空清单', async ({ page }) => {
  await page.goto('/e2e/fixtures/onboarding.html?host=failed');
  await page.getByRole('button', { name: /Local agents/u }).click();
  await expect(page.getByText('fixture.external_action_unavailable')).toBeVisible();
  await expect(page.getByText(/No agent task has opened/u)).toHaveCount(0);
});
