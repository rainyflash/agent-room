import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import type { MyAgentsFixtureWindow } from '../src/test/my-agents-fixture-controls';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const appNames = /Codex|Claude Code|Cursor/u;
const invite = { name: 'Bring an agent', exact: true } as const;

for (const width of [1440, 390]) {
  test(`接入 Agent 一屏完成：复制一段话，它进来就显示 ${String(width)}px`, async ({
    page,
    context,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/e2e/fixtures/my-agents.html');
    // “我的 Agent”页头的主按钮就是接入；这台电脑上还没有 Agent 进来。
    await expect(page.getByText('No agent on this computer has joined yet.')).toBeVisible();
    await page.getByRole('main').getByRole('button', invite).click();
    const dialog = page.getByRole('dialog', { name: 'Bring an agent' });
    // 三种通用方式平级，默认网络接入；对话框里不出现任何具体 Agent 应用的名字。
    await expect(dialog.getByRole('radio', { name: /^Network/u })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeVisible();
    // 没有房间又走网络：看不到它进来，如实说它去公共大厅。
    await expect(dialog.getByText(/It joins the public lobby; look for it there/u)).toBeVisible();
    await expect(dialog).not.toContainText(appNames);

    await dialog.getByRole('radio', { name: /^Command line/u }).click();
    await expect(dialog).not.toContainText(appNames);
    await expect(dialog.getByText(/It shows up here when it joins/u)).toBeVisible();
    await dialog.getByRole('button', { name: 'Copy message' }).click();
    await expect(
      dialog.getByRole('button', { name: 'Copied. Send it to your agent.' }),
    ).toBeVisible();
    await expect(dialog.getByText('Waiting for your agent to join…')).toBeVisible();
    const clipboard = await page.evaluate(() => navigator.clipboard.readText());
    // 一段话就够：按安装路径运行 join，再读 guide；不带一次性邀请，也不用记人物编号。
    expect(clipboard).toContain(String.raw`& 'C:\Agent Room\agent-room.exe' join`);
    expect(clipboard).toContain(String.raw`& 'C:\Agent Room\agent-room.exe' guide`);
    expect(clipboard).toContain('untrusted input');
    expect(clipboard).not.toMatch(/--invite|--profile/u);
    expect(clipboard).not.toMatch(/Codex|Claude Code|Cursor|CODEX_/u);

    // 本机方式时对话框把一个人物挂在连接服务上，说一句“接入”的 Agent 直接接走它。
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as MyAgentsFixtureWindow).__agentRoomFixtureControls.parkedInvitation(),
        ),
      )
      .not.toBeNull();
    await page.evaluate(() => {
      (window as MyAgentsFixtureWindow).__agentRoomFixtureControls.arriveAgent();
    });
    await expect(dialog.getByText('“Scout” joined')).toBeVisible();
    await expect(dialog.getByText('It’s reading messages.')).toBeVisible();
    await expect(dialog.getByText('Waiting for your agent to join…')).toHaveCount(0);
    // 接走后不再挂着同一个人物。
    await expect
      .poll(() =>
        page.evaluate(() =>
          (window as MyAgentsFixtureWindow).__agentRoomFixtureControls.parkedInvitation(),
        ),
      )
      .toBeNull();
    await page.screenshot({ path: testInfo.outputPath(`agent-invite-${String(width)}.png`) });
    await expectNoHorizontalOverflow(page);
    // 宽屏居中，手机上贴底；三个方式的名字都完整显示，不被截断。
    const box = await dialog.boundingBox();
    expect(box).not.toBeNull();
    if (box !== null && width >= 768)
      expect(Math.abs(box.x + box.width / 2 - width / 2)).toBeLessThan(2);
    if (box !== null && width < 768) expect(Math.abs(box.y + box.height - 900)).toBeLessThan(2);
    const clipped = await dialog
      .locator('.ar-segmented__label')
      .evaluateAll(
        (labels) => labels.filter((label) => label.scrollWidth > label.clientWidth).length,
      );
    expect(clipped).toBe(0);
    // 对话框、分段选择、提示、复制区和到达列表一起过一遍无障碍检查。
    const scan = await new AxeBuilder({ page })
      .include('.ar-dialog')
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(
      scan.violations.map(({ id, nodes }) => ({
        id,
        nodes: nodes.map(({ target, failureSummary }) => ({ target, failureSummary })),
      })),
    ).toEqual([]);
    await dialog.getByRole('button', { name: 'Done' }).click();
    await expect(dialog).toHaveCount(0);

    // 它也出现在“这台电脑上的 Agent”里。再打开时上次选的命令行还在；已经在的 Scout 不算新来的。
    // 列表每 5 秒读一次本机会话。
    await expect(
      page.getByRole('region', { name: 'Agents on this computer' }).getByText('Scout'),
    ).toBeVisible({ timeout: 10_000 });
    await page.getByRole('main').getByRole('button', invite).click();
    await expect(dialog.getByRole('radio', { name: /^Command line/u })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await expect(dialog.getByText(/It shows up here when it joins/u)).toBeVisible();
    await expect(dialog.getByText('“Scout” joined')).toHaveCount(0);
    await dialog.getByRole('button', { name: 'Close' }).click();
    await expect(dialog).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}

test('读不到本机会话时如实说，不伪造空清单', async ({ page }) => {
  await page.goto('/e2e/fixtures/my-agents.html?host=failed');
  const sessions = page.getByRole('region', { name: 'Agents on this computer' });
  await expect(sessions.getByText('Can’t check this computer’s agents right now.')).toBeVisible();
  await expect(sessions.getByText('No agent on this computer has joined yet.')).toHaveCount(0);
  // 错误码收在详情里，要排查时展开。
  await sessions.getByText('Connection and identity details').click();
  await expect(sessions.getByText('fixture.external_action_unavailable')).toBeVisible();
  await page.getByRole('main').getByRole('button', invite).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('radio', { name: /^Command line/u }).click();
  await expect(dialog.getByText('Can’t check this computer’s agents right now.')).toBeVisible();
  // 读不到会话不挡着复制：Agent 照样能进来。
  await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeEnabled();
});

test('连接服务重连时只是告知，不挡复制；重试后提示消失', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/my-agents.html?bridge=reconnecting');
  await expect(page.getByRole('link', { name: 'This computer: Connecting' })).toBeVisible();
  await page.getByRole('main').getByRole('button', invite).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('radio', { name: /^Command line/u }).click();
  await expect(dialog.getByText(/Reconnecting automatically/u)).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeEnabled();
  await dialog.getByRole('button', { name: 'Retry connection' }).click();
  await expect(dialog.getByText(/Reconnecting automatically/u)).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeEnabled();
  await expect(dialog.getByText(/Finish authorization/u)).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('MCP 只给同一份通用配置：这台电脑的设置和接入对话框都不出现具体应用', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/my-agents.html?bridge=authorized');
  const mcpHint = /Add this JSON to the tool’s MCP configuration/u;
  const thisComputer = page.getByRole('region', { name: 'This computer', exact: true });
  await thisComputer.getByText('MCP compatibility').click();
  await expect(thisComputer.getByText(mcpHint)).toBeVisible();
  await expect(thisComputer.getByText(/agent-room-mcp\.exe/u)).toBeVisible();
  await expect(thisComputer.getByRole('button', { name: 'Copy JSON' })).toBeVisible();
  await expect(page.getByRole('main')).not.toContainText(appNames);

  await page.getByRole('main').getByRole('button', invite).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('radio', { name: /^MCP/u }).click();
  // 配好的 Agent 只要听到一句“接入 Agent Room”；第一次用才要展开配置。
  await expect(dialog.getByText(/just tell your agent “Join Agent Room”/u)).toBeVisible();
  await expect(dialog.getByText(mcpHint)).toBeHidden();
  await dialog.getByText('First time using MCP? Set it up once').click();
  await expect(dialog.getByText(mcpHint)).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Copy JSON' })).toBeVisible();
  await expect(dialog).not.toContainText(appNames);
  await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeEnabled();
  await dialog.getByRole('radio', { name: /^Command line/u }).click();
  await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeEnabled();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});
