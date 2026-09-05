import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`大厅直接聊天、提及和回复：${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto('/e2e/fixtures/lobby-scene.html');
    const panel = page.getByRole('region', { name: 'Conversation', exact: true });
    await expect(panel).toBeVisible();
    const input = panel.getByRole('textbox', { name: 'Message', exact: true });
    const mention = panel.getByRole('combobox', { name: 'Mention an agent' });
    await expect(mention.locator('option')).not.toHaveCount(1);
    await mention.selectOption({ index: 1 });
    await input.fill('Can we discuss this together?');
    await input.press('Enter');
    const log = panel.getByRole('log');
    await expect(log).toContainText('Can we discuss this together?');
    await expect(input).toHaveValue('');
    await log.getByRole('button', { name: 'Reply to Fixture operator' }).click();
    await input.fill('Here is a follow-up.');
    await panel.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(log.locator('article')).toHaveCount(2);
    await expect(log.locator('blockquote')).toHaveText(
      'Fixture operator: Can we discuss this together?',
    );
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
    await page.screenshot({
      path: testInfo.outputPath(`conversation-${String(width)}.png`),
      fullPage: true,
    });
  });
}

for (const width of [1440, 390]) {
  test(`私聊共用输入且拉黑后禁止发送：${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/e2e/fixtures/lobby-scene.html');
    if (width === 390)
      await page.getByRole('button', { name: 'Open conversations', exact: true }).click();
    await page.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
    const direct = page.locator('.direct-conversation');
    const input = direct.getByRole('textbox', { name: 'Message', exact: true });
    await expect(input).toBeVisible();
    await input.fill('A private question.');
    await direct.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(direct.getByRole('log')).toContainText('A private question.');
    await direct.getByRole('button', { name: 'Block', exact: true }).click();
    await expect(input).toBeDisabled();
    await direct.getByRole('button', { name: 'Unblock', exact: true }).click();
    await expect(input).toBeEnabled();
    await expect(
      direct.getByRole('heading', { name: 'Build Agent 002', exact: true }),
    ).toBeInViewport();
    await expect(input).toBeInViewport();
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
    await page.screenshot({
      path: testInfo.outputPath(`direct-${String(width)}.png`),
    });
  });
}
