import { test, expect, type Locator, type Page } from '@playwright/test';
import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openAgentRoster } from './support/workspace-navigation';

/** Agent 表格里名字以这个开头的那一行。 */
function agentRow(page: Page, roster: Locator, name: string): Locator {
  return roster
    .getByRole('row')
    .filter({ has: page.getByRole('button', { name: new RegExp(`^${name}`, 'u') }) });
}

/** 表格从上到下的名字。 */
async function rowOrder(roster: Locator): Promise<readonly string[]> {
  return await roster.locator('tbody .agent-table__label').allTextContents();
}

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
    const roster = await openAgentRoster(page);
    const state = (name: string) => agentRow(page, roster, name).locator('.agent-table__state');
    // 状态一列一直带字；离线的写离线了多久，“离线”两个字留给读屏。
    await expect(state('Build Agent 001')).toHaveText('Waiting');
    await expect(state('Build Agent 002')).toHaveText('Reads on next run');
    await expect(state('Build Agent 003')).toHaveText(/^Offline · \d+ minutes ago$/u);
    await expect(state('Build Agent 004')).toHaveText(/^Offline · \d+ hours ago$/u);
    await expect(state('Build Agent 005')).toHaveText(/^Offline · \d+ days ago$/u);
    // 不画分组标题：在等的在前，然后下次运行时读，离线的最近离线的在前。
    const order = await rowOrder(roster);
    const positions = [1, 2, 3, 4, 5].map((index) =>
      order.indexOf(`Build Agent 00${String(index)}`),
    );
    expect(positions).not.toContain(-1);
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
    // 离线太久的收进归档，不算在个数里。
    await expect(roster.getByRole('button', { name: /^Build Agent 006/u })).toHaveCount(0);
    await expect(roster.getByRole('tab', { name: 'Agents 23', exact: true })).toBeVisible();
    // 表格下面滚出去的那几行不把整页撑高：整页不滚。
    expect(
      await page.locator('main').evaluate((main) => main.scrollHeight - main.clientHeight),
    ).toBe(0);
    await page.screenshot({ path: info.outputPath(`members-${String(width)}.png`) });
    await roster.getByRole('button', { name: 'Archived (1)', exact: true }).click();
    await expect(roster.getByRole('button', { name: /^Build Agent 006/u })).toBeVisible();
    await expect(state('Build Agent 006')).toHaveText(/^Offline · /u);
    await expect(roster.getByRole('button', { name: /^Build Agent 001/u })).toHaveCount(0);
    await page.evaluate(() => {
      (window as LobbyFixtureWindow).__agentRoomFixtureControls.setAgentReception(
        '01990d9e-8400-7000-8000-000000000006',
        'waiting',
      );
    });
    // 归档的回来了：自己回到成员那一页，没有东西可归档时不留那个按钮。
    await expect(roster.getByRole('button', { name: /^(Archived|Members) \(/u })).toHaveCount(0);
    await expect(state('Build Agent 006')).toHaveText('Waiting');
    await expect(roster.getByRole('tab', { name: 'Agents 24', exact: true })).toBeVisible();
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
  const roster = await openAgentRoster(page);
  const state = (name: string) => agentRow(page, roster, name).locator('.agent-table__state');
  await expect(state('Build Agent 001')).toHaveText('Waiting');
  await expect(state('Build Agent 002')).toHaveText('Reads on next run');
  // 离线多久照它被看到离线的那一刻算，不是名片写的时间。
  await expect(state('Build Agent 003')).toHaveText('Offline · 20 minutes ago');
  await expect(roster.locator('.room-roster__summary > span')).toHaveText([
    '8 waiting',
    '8 offline',
  ]);
  await expect(roster.getByRole('button', { name: /^Archived \(/u })).toHaveCount(0);
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
  const roster = await openAgentRoster(page);
  const archived = roster.getByRole('button', { name: /^Archived \(/u });
  await expect(roster.getByRole('button', { name: /^Build Agent 006/u })).toBeVisible();
  await expect(archived).toHaveCount(0);
  // 规则在表格最下面。
  await roster.getByText('Offline archiving', { exact: true }).click();
  await roster.getByRole('combobox', { name: 'Archive offline agents after' }).selectOption('1');
  await roster.getByRole('button', { name: 'Save rule', exact: true }).click();
  await expect(archived).toHaveText('Archived (1)');
  await expect(roster.getByRole('button', { name: /^Build Agent 006/u })).toHaveCount(0);
  await archived.click();
  await expect(roster.getByRole('button', { name: /^Build Agent 006/u })).toBeVisible();
  await roster.getByText('Offline archiving', { exact: true }).click();
  await roster.getByRole('combobox', { name: 'Archive offline agents after' }).selectOption('30');
  await roster.getByRole('button', { name: 'Save rule', exact: true }).click();
  // 规则放宽以后它回到成员里，是同一个 Agent。
  await expect(roster.getByRole('button', { name: /^(Archived|Members) \(/u })).toHaveCount(0);
  await expect(roster.getByRole('button', { name: /^Build Agent 006/u })).toBeVisible();
});
