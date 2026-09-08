import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`接待任务登记、启停和维护 ${String(width)}px`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 1000 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/e2e/fixtures/onboarding.html?host=ready&reception=1');
    await page.getByRole('button', { name: /Local agents/u }).click();
    const panel = page.getByRole('region', { name: 'Reception tasks' });
    await panel.getByLabel('Reply authorization').selectOption({ index: 1 });
    await panel.getByRole('button', { name: 'Add reception task' }).click();
    const card = panel.getByRole('article');
    await expect(card.getByText('Reception Scout')).toBeVisible();
    await card.getByRole('button', { name: 'Start reception' }).click();
    await expect(card.getByText('Waiting for your mention')).toBeVisible();
    await card.getByRole('button', { name: 'Pause', exact: true }).click();
    await expect(card.getByText('Paused', { exact: true })).toBeVisible();
    await card.getByText('Authorization and paths', { exact: true }).click();
    await card.getByLabel('Workspace', { exact: true }).fill('C:/Projects/Studio updated');
    await card.getByRole('button', { name: 'Save authorization and paths' }).click();
    await expect(card.getByText('C:/Projects/Studio updated', { exact: true })).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await panel.screenshot({ path: testInfo.outputPath(`reception-${String(width)}.png`) });
    await card.getByRole('button', { name: 'Remove', exact: true }).click();
    await expect(card).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}

test('未确认回复须核对回执且不能直接移除', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/onboarding.html?host=ready&reception=pending');
  await page.getByRole('button', { name: /Local agents/u }).click();
  const panel = page.getByRole('region', { name: 'Reception tasks' });
  await expect(panel.getByText('receiver.reply_unconfirmed')).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Remove', exact: true })).toBeDisabled();
  await panel.getByRole('button', { name: 'Verify receipt' }).click();
  await expect(panel.getByText('Reply confirmed in room')).toBeVisible();
  await expect(panel.getByRole('button', { name: 'Start reception' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await panel.screenshot({ path: testInfo.outputPath('reception-reconciled.png') });
  expect(failures).toEqual([]);
});
