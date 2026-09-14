import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`刷新后保留当前房间和未发送的草稿 ${String(width)}px`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/e2e/fixtures/lobby-scene.html?view=conversation');
    const input = page.getByRole('textbox', { name: 'Message', exact: true });
    await input.fill('继续这段讨论，刷新后不要丢失。');
    await page.reload();
    await expect(input).toHaveValue('继续这段讨论，刷新后不要丢失。');
    await expect(input).toBeEnabled();
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    const sent = page.getByRole('article').filter({ hasText: '继续这段讨论，刷新后不要丢失。' });
    await expect(sent.getByText('Sent to room', { exact: true })).toBeVisible();
    await expect(sent.getByText('Agent receipt unconfirmed', { exact: true })).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath(`restored-draft-${String(width)}.png`) });
    expect(failures).toEqual([]);
  });
}
