import { test, expect } from '@playwright/test';
import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`Agent reception, offline age and archive recovery at ${String(width)}px`, async ({
    page,
  }, info) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 960 });
    await page.goto('/e2e/fixtures/lobby-scene.html?agents=24');
    await page.evaluate(() => {
      const controls = (window as LobbyFixtureWindow).__agentRoomFixtureControls;
      const id = (index: number) => `01990d9e-8400-7000-8000-${String(index).padStart(12, '0')}`;
      controls.setAgentReception(id(1), 'waiting');
      controls.setAgentReception(id(2), 'on_resume');
      controls.setAgentReception(id(3), 'offline', 5 * 60_000);
      controls.setAgentReception(id(4), 'offline', 4 * 3_600_000);
      controls.setAgentReception(id(5), 'offline', 2 * 86_400_000);
      controls.setAgentReception(id(6), 'offline', 8 * 86_400_000);
    });
    await page.getByRole('button', { name: 'View agents', exact: true }).click();
    const roster = page.locator('.workspace-members .list-roster');
    for (const name of [
      'Online · waiting for messages',
      'Online · reads on next run',
      'Offline · under 1 hour',
      'Offline · 1–24 hours',
      'Offline · 1–7 days',
    ]) {
      await expect(roster.getByRole('heading', { name })).toBeVisible();
    }
    await expect(roster.getByRole('button', { name: /^Build Agent 006/u })).toHaveCount(0);
    await page.screenshot({ path: info.outputPath(`members-${String(width)}.png`) });
    await roster.getByRole('button', { name: 'Archived (1)' }).click();
    await expect(roster.getByRole('button', { name: /^Build Agent 006/u })).toBeVisible();
    await expect(roster.getByText('Offline · 7 days or more')).toBeVisible();
    await page.evaluate(() => {
      (window as LobbyFixtureWindow).__agentRoomFixtureControls.setAgentReception(
        '01990d9e-8400-7000-8000-000000000006',
        'waiting',
      );
    });
    await expect(roster.getByRole('button', { name: 'Archived (0)' })).toBeVisible();
    await roster.getByRole('button', { name: /^Members \(/u }).click();
    await roster.getByRole('button', { name: /^Build Agent 006/u }).click();
    await expect(page.locator('.agent-inspector')).toContainText('Online · waiting for messages');
    // 工作状态和任务摘要不再显示（specs/agent-liveness）。
    await expect(page.locator('.agent-inspector')).not.toContainText('What it is doing');
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
  });
}

test('Agents that only write a card follow their Matrix presence', async ({ page }) => {
  const failures = collectPageFailures(page);
  // 名片都是三天前进房间时写的：依次在等消息、下次运行时读、20 分钟前离线。
  await page.goto('/e2e/fixtures/lobby-scene.html?agents=24&cards');
  await page.getByRole('button', { name: 'View agents', exact: true }).click();
  const roster = page.locator('.workspace-members .list-roster');
  for (const name of [
    'Online · waiting for messages',
    'Online · reads on next run',
    'Offline · under 1 hour',
  ]) {
    await expect(roster.getByRole('heading', { name })).toBeVisible();
  }
  await expect(roster.getByRole('button', { name: 'Archived (0)' })).toBeVisible();
  await roster.getByRole('button', { name: /^Build Agent 003/u }).click();
  const inspector = page.locator('.agent-inspector');
  await expect(inspector).toContainText('Under 1 hour');
  await expect(inspector).toContainText('Last connected');
  expect(failures).toEqual([]);
});

test('Room managers can change the shared archive rule and keep the same identities', async ({
  page,
}) => {
  await page.goto('/e2e/fixtures/lobby-scene.html?agents=24');
  await page.evaluate(() => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.setAgentReception(
      '01990d9e-8400-7000-8000-000000000006',
      'offline',
      2 * 86_400_000,
    );
  });
  await page.getByRole('button', { name: 'View agents', exact: true }).click();
  const roster = page.locator('.workspace-members');
  await expect(roster.getByRole('button', { name: 'Archived (0)' })).toBeVisible();
  await roster.getByText('Offline archiving', { exact: true }).click();
  await roster.getByRole('combobox', { name: 'Archive offline agents after' }).selectOption('1');
  await roster.getByRole('button', { name: 'Save rule', exact: true }).click();
  await expect(roster.getByRole('button', { name: 'Archived (1)' })).toBeVisible();
  await roster.getByRole('button', { name: 'Archived (1)' }).click();
  await expect(roster.getByRole('button', { name: /^Build Agent 006/u })).toBeVisible();
  await roster.getByText('Offline archiving', { exact: true }).click();
  await roster.getByRole('combobox', { name: 'Archive offline agents after' }).selectOption('30');
  await roster.getByRole('button', { name: 'Save rule', exact: true }).click();
  await expect(roster.getByRole('button', { name: 'Archived (0)' })).toBeVisible();
});
