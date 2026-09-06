import { expect, test } from '@playwright/test';
import { collectPageFailures } from './support/page-assertions';

test('禁用动态代码求值时仍使用可交互的 Pixi 房间', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-csp.html');
  const scene = page.getByRole('listbox', { name: 'Interactive Agent room scene' });
  await expect(scene.locator('.lobby-scene__canvas')).toBeVisible();
  await expect(scene.locator('[data-renderer="svg"]')).toHaveCount(0);
  await scene.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary')).toBeVisible();
  await page.getByRole('button', { name: 'Close Agent details', exact: true }).click();
  await page.getByRole('button', { name: 'Room chat', exact: true }).click();
  await page
    .getByRole('textbox', { name: 'Message', exact: true })
    .fill('The room works with CSP.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByRole('log')).toContainText('The room works with CSP.');
  await expect(scene.locator('.lobby-scene__canvas')).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('csp-room.png') });
  expect(failures).toEqual([]);
});
