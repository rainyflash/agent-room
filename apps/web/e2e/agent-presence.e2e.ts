import { expect, test } from '@playwright/test';

import { collectPageFailures } from './support/page-assertions';

// “我的 Agent”里在不在线看 Matrix 的在线状态，和房间页一个依据（specs/agent-liveness/design.md）。
// 夹具 Agent 的连接上一直记着在线（发过自动回复的就这样），?presence= 给它一个在线状态。
test('我的 Agent：连接上记着在线、Matrix 说它离线时，说上次在线', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/my-agents.html?presence=offline');

  const card = page.getByRole('button', { name: /Studio companion/u });
  await expect(card).toContainText('Last online');
  await expect(card).not.toContainText('Online now');
  await card.click();
  const dialog = page.getByRole('dialog', { name: 'Studio companion' });
  await expect(dialog.getByRole('region', { name: 'Where it runs' })).toContainText('Last online');
  expect(failures).toEqual([]);
});

test('我的 Agent：连着、没在等消息也算在线', async ({ page }) => {
  await page.goto('/e2e/fixtures/my-agents.html?presence=unavailable');
  await expect(page.getByRole('button', { name: /Studio companion/u })).toContainText('Online now');
});
