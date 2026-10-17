import { expect, test } from '@playwright/test';

import { collectPageFailures } from './support/page-assertions';

// “我的 Agent”里在不在线看 Matrix 的在线状态，和房间页一个依据（specs/agent-liveness/design.md）。
// 夹具 Agent 的连接上一直记着在线（发过自动回复的就这样），?presence= 给它一个在线状态。
// 在线的一组摊开，不在线的收起来、只写有几个（维护者 2026-10-17 定）。
test('我的 Agent：连接上记着在线、Matrix 说它离线时，收在不在线里、说上次在线', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/my-agents.html?presence=offline');

  await expect(page.getByRole('heading', { name: 'Online · 0' })).toBeVisible();
  await expect(page.getByText('None of your agents are online right now.')).toBeVisible();
  const card = page.getByRole('button', { name: /Studio companion/u });
  await expect(card).toBeHidden();
  await page.getByText('Offline · 1').click();
  await expect(card).toContainText('Last online');
  await expect(card).not.toContainText('Online now');
  await card.click();
  const dialog = page.getByRole('dialog', { name: 'Studio companion' });
  await expect(dialog.getByRole('region', { name: 'Where it runs' })).toContainText('Last online');
  expect(failures).toEqual([]);
});

test('我的 Agent：连着、没在等消息也算在线，摊在在线那一组', async ({ page }) => {
  await page.goto('/e2e/fixtures/my-agents.html?presence=unavailable');
  const online = page.getByRole('region', { name: 'Online · 1' });
  await expect(online.getByRole('button', { name: /Studio companion/u })).toContainText(
    'Online now',
  );
  await expect(page.getByText(/^Offline ·/u)).toHaveCount(0);
});
