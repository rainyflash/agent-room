import { expect, test, type Locator, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html';

test('已验证正文必须经过精确实例授权才能成为一次性上下文', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });

  const inspector = await openInspector(page);
  await expect(inspector.getByRole('button', { name: 'Give to Agent' })).toHaveCount(0);

  await inspector.getByRole('button', { name: 'Open full content' }).click();
  await inspector.getByRole('button', { name: 'Give to Agent' }).click();

  const panel = inspector.getByRole('region', { name: 'Approve one-time context' });
  const targets = panel.getByRole('radio', { name: /desktop/u });
  await expect(targets).toHaveCount(2);
  await expect(panel.getByText('Local Codex Agent', { exact: true })).toBeVisible();
  await expect(panel.getByText('Research Agent', { exact: true })).toBeVisible();
  await expect(panel.getByText('Online · deliver now', { exact: true })).toBeVisible();
  await expect(panel.getByText('Offline · queue', { exact: true })).toBeVisible();
  await panel.getByRole('radio', { name: /claude-desktop/u }).click();
  await expect(
    panel.getByText('Queue until this instance reconnects', { exact: true }),
  ).toBeVisible();
  await expect(panel.getByRole('checkbox', { name: 'Read verified text' })).toBeDisabled();
  await expect(panel.getByRole('radio', { name: 'Summarize' })).toBeChecked();
  await expect(panel.getByRole('radio', { name: '15 min' })).toBeChecked();

  await panel.getByRole('button', { name: 'Confirm handoff' }).click();
  await expect(panel.getByText('Queued for one instance', { exact: true })).toBeVisible();
  await panel.getByRole('button', { name: 'Check status' }).click();
  await expect(panel.getByText('Claimed by target runtime', { exact: true })).toBeVisible();
  await panel.getByRole('button', { name: 'Revoke access' }).click();
  await expect(panel.getByText('Access revoked', { exact: true })).toBeVisible();

  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('窄屏授权面板保持完整可操作且不会横向溢出', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });

  const inspector = await openInspector(page);
  await inspector.getByRole('button', { name: 'Open full content' }).click();
  await inspector.getByRole('button', { name: 'Give to Agent' }).click();

  const panel = inspector.getByRole('region', { name: 'Approve one-time context' });
  await panel.getByRole('radio', { name: '15 min' }).click();
  await expect(panel.getByRole('button', { name: 'Confirm handoff' })).toBeVisible();
  await panel.getByRole('button', { name: 'Confirm handoff' }).click();
  await expect(panel.getByText('Queued for one instance', { exact: true })).toBeVisible();

  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

async function openInspector(page: Page): Promise<Locator> {
  await page.goto(fixturePath);
  await page.getByRole('button', { name: 'Expand signal timeline' }).click();
  await page
    .getByRole('list', { name: 'Room signal timeline' })
    .getByRole('button', { name: /Protocol review ready/u })
    .click();
  const inspector = page.getByRole('complementary', { name: 'Protocol review ready' });
  await expect(inspector).toBeVisible();
  return inspector;
}
