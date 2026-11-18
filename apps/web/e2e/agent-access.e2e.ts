import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Browser } from '@playwright/test';
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
    // 三种通用方式按命令行、MCP、网络排，桌面端从命令行开始；对话框里不出现任何具体 Agent 应用的名字。
    await expect(dialog.getByRole('radio')).toHaveText(['Command line', 'MCP', 'Network']);
    await expect(dialog.getByRole('radio', { name: /^Command line/u })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await expect(dialog).not.toContainText(appNames);
    // 没有房间又走网络：看不到它进来，如实说它去公共大厅。
    await dialog.getByRole('radio', { name: /^Network/u }).click();
    await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeVisible();
    await expect(dialog.getByText(/It joins the public lobby; look for it there/u)).toBeVisible();
    await expect(dialog).not.toContainText(appNames);

    await dialog.getByRole('radio', { name: /^Command line/u }).click();
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
    expect(clipboard).toContain('chat with everyone');
    expect(clipboard).not.toMatch(/untrusted|only follow my instructions/u);
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

test('接入时默认开后台回复：话里请它登记，它登记了对话框就替人打开', async ({
  page,
  context,
}, testInfo) => {
  const failures = collectPageFailures(page);
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.setViewportSize({ width: 390, height: 900 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/e2e/fixtures/my-agents.html?reception=1');
  await page.getByRole('main').getByRole('button', invite).click();
  const dialog = page.getByRole('dialog', { name: 'Bring an agent' });
  const option = dialog.getByRole('checkbox', { name: 'Let it reply when you’re away' });
  await expect(option).toBeChecked();
  await expect(dialog.getByText(/Everyone in the room sees its replies/u)).toBeVisible();

  const copied = async () => {
    await dialog.getByRole('button', { name: /^Cop/u }).click();
    return await page.evaluate(() => navigator.clipboard.readText());
  };
  const cli = await copied();
  expect(cli).toContain('also run register as the join result says');
  expect(cli).not.toMatch(appNames);
  await dialog.getByRole('radio', { name: /^MCP/u }).click();
  await expect.poll(copied).toContain('agent_room_register_reception');

  // 它进来以后登记了这间房的后台回复：对话框接着替人打开，不用再去“我的 Agent”。
  await page.evaluate(() => {
    (window as MyAgentsFixtureWindow).__agentRoomFixtureControls.arriveAgent({ registers: true });
  });
  await expect(dialog.getByText('“Scout” joined')).toBeVisible();
  await expect(dialog.getByText('Background replies are on for 30 days.')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await dialog.screenshot({ path: testInfo.outputPath('agent-invite-background.png') });
  await dialog
    .locator('.agent-invite__arrivals')
    .screenshot({ path: testInfo.outputPath('agent-invite-background-arrived.png') });
  const scan = await new AxeBuilder({ page })
    .include('.ar-dialog')
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(scan.violations.map(({ id }) => id)).toEqual([]);
  await dialog.getByRole('button', { name: 'Done' }).click();

  // “我的 Agent”的后台回复里就是它，正在替它听着。
  const panel = page.getByRole('region', { name: 'Background replies' });
  await expect(panel.getByRole('article').getByText('Waiting for messages for it')).toBeVisible({
    timeout: 10_000,
  });
  expect(failures).toEqual([]);
});

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

test('托管的连接服务等电脑授权时只说正在连接，退回设备码才出授权提示', async ({ page }) => {
  const failures = collectPageFailures(page);
  // 桌面端拿应用里的登录换电脑授权交给连接服务（specs/desktop-sign-in），这段时间是 starting。
  await page.goto('/e2e/fixtures/my-agents.html?bridge=starting');
  await expect(page.getByRole('link', { name: 'This computer: Connecting' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open secure sign-in' })).toHaveCount(0);

  await page.goto('/e2e/fixtures/my-agents.html?bridge=authorization_required');
  await expect(page.getByRole('button', { name: 'Open secure sign-in' }).first()).toBeVisible();
  expect(failures).toEqual([]);
});

test('MCP 只给同一份通用配置：设置里的“这台电脑”和接入对话框都不出现具体应用', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/my-agents.html?bridge=authorized&settings=this-computer');
  const mcpHint = /Add this JSON to the tool’s MCP configuration/u;
  const settings = page.getByRole('region', { name: 'This computer', exact: true });
  await settings.getByText('MCP compatibility').click();
  await expect(settings.getByText(mcpHint)).toBeVisible();
  await expect(settings.getByText(/agent-room-mcp\.exe/u)).toBeVisible();
  await expect(settings.getByRole('button', { name: 'Copy JSON' })).toBeVisible();
  await expect(page.getByRole('main')).not.toContainText(appNames);

  await page.goto('/e2e/fixtures/my-agents.html?bridge=authorized');
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

const WINDOWS_USER_AGENT =
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36';
const LINUX_USER_AGENT =
  'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36';
const WINDOWS_INSTALLER = 'https://download.fixture.invalid/agent-room.exe';

/**
 * 网页上的访客：系统写明；开发服务器的运行时清单里没有安装包地址，这里补一个 Windows 的。
 * 网页端走到的几步是真发给控制面的，夹具的控制面地址不通，这里接住、记下报了哪几步。
 */
async function webVisitor(browser: Browser, userAgent: string, width: number) {
  const context = await browser.newContext({
    reducedMotion: 'reduce',
    userAgent,
    viewport: { width, height: 900 },
  });
  await context.route('**/runtime-manifest.json', async (route) => {
    const response = await route.fetch();
    const manifest = (await response.json()) as Record<string, unknown>;
    await route.fulfill({ response, json: { ...manifest, windowsDownloadUrl: WINDOWS_INSTALLER } });
  });
  const steps: unknown[] = [];
  await context.route('https://api.fixture.invalid/telemetry/onboarding', async (route) => {
    const request = route.request();
    if (request.method() === 'POST') steps.push(request.postDataJSON());
    await route.fulfill({
      status: 204,
      headers: {
        'Access-Control-Allow-Credentials': 'true',
        'Access-Control-Allow-Headers': 'content-type',
        'Access-Control-Allow-Origin': (await request.headerValue('origin')) ?? '*',
      },
    });
  });
  return { context, steps };
}

// 网页上没选过方式时：访客这个系统有安装包就从命令行开始，头一步是装桌面应用；不想装的旁边换成网络。
for (const width of [1280, 390]) {
  test(`网页上接入 Agent：先请人装桌面应用，不装的一步换成网络 ${String(width)}px`, async ({
    browser,
  }, testInfo) => {
    const { context, steps } = await webVisitor(browser, WINDOWS_USER_AGENT, width);
    const page = await context.newPage();
    const failures = collectPageFailures(page);
    await page.goto('/e2e/fixtures/my-agents.html?browser&computers=none');
    await page.getByRole('main').getByRole('button', invite).click();
    const dialog = page.getByRole('dialog', { name: 'Bring an agent' });
    await expect(dialog.getByRole('radio', { name: /^Command line/u })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await expect(
      dialog.getByRole('heading', { name: 'First, install Agent Room on your agent’s computer' }),
    ).toBeVisible();
    // 明黄的按钮只有下载这一个；它在新标签页里下，对话框留着。
    const download = dialog.getByRole('link', { name: 'Download Agent Room' });
    await expect(download).toHaveAttribute('href', WINDOWS_INSTALLER);
    await expect(download).toHaveAttribute('target', '_blank');
    await expect(dialog.locator('.ar-button--primary')).toHaveCount(1);
    await expect(download).toHaveClass(/ar-button--primary/u);
    await expect(dialog).not.toContainText(appNames);
    await expectNoHorizontalOverflow(page);
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
    await page.screenshot({ path: testInfo.outputPath(`web-invite-install-${String(width)}.png`) });

    // 点了下载：下载退成灰的，轮到复制。
    await context.route(WINDOWS_INSTALLER, (route) => route.abort());
    const opened = page.waitForEvent('popup');
    await download.click();
    await (await opened).close();
    await expect(dialog.getByText(/The download has started/u)).toBeVisible();
    // 打开对话框、点下载各报一步，只有步子的名字和“网页”。
    await expect
      .poll(() => steps)
      .toEqual([
        { step: 'invite_opened', surface: 'web' },
        { step: 'desktop_download', surface: 'web' },
      ]);
    await expect(dialog.locator('.ar-button--primary')).toHaveCount(1);
    await expect(dialog.getByRole('button', { name: 'Copy message' })).toHaveClass(
      /ar-button--primary/u,
    );
    await page.screenshot({ path: testInfo.outputPath(`web-invite-started-${String(width)}.png`) });

    // 不装应用：一步换成网络，下次打开还是网络。
    await dialog.getByRole('button', { name: 'Use Network instead, nothing to install' }).click();
    await expect(dialog.getByRole('radio', { name: /^Network/u })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await expect(dialog.getByText(/Nothing to install: an agent that can send web/u)).toBeVisible();
    await expect(dialog.getByRole('button', { name: 'Copy message' })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath(`web-invite-network-${String(width)}.png`) });
    await dialog.getByRole('button', { name: 'Close' }).click();
    await page.getByRole('main').getByRole('button', invite).click();
    await expect(dialog.getByRole('radio', { name: /^Network/u })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    expect(failures).toEqual([]);
    await context.close();
  });
}

test('网页上接入 Agent：账号已经有装着 Agent Room 的电脑时不再请人先装', async ({
  browser,
}, testInfo) => {
  const { context } = await webVisitor(browser, WINDOWS_USER_AGENT, 1280);
  const page = await context.newPage();
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/my-agents.html?browser');
  await page.getByRole('main').getByRole('button', invite).click();
  const dialog = page.getByRole('dialog', { name: 'Bring an agent' });
  await expect(dialog.getByRole('radio', { name: /^Command line/u })).toHaveAttribute(
    'aria-checked',
    'true',
  );
  await expect(
    dialog.getByText(/This account already has a computer with Agent Room/u),
  ).toBeVisible();
  await expect(dialog.getByRole('heading', { name: /install Agent Room/u })).toHaveCount(0);
  await expect(dialog.getByRole('button', { name: 'Copy message' })).toHaveClass(
    /ar-button--primary/u,
  );
  await expect(dialog.locator('.ar-button--primary')).toHaveCount(1);
  await page.screenshot({ path: testInfo.outputPath('web-invite-has-computer.png') });
  expect(failures).toEqual([]);
  await context.close();
});

test('网页上接入 Agent：装不了桌面应用的系统还从网络开始', async ({ browser }) => {
  const { context } = await webVisitor(browser, LINUX_USER_AGENT, 1280);
  const page = await context.newPage();
  const failures = collectPageFailures(page);
  await page.goto('/e2e/fixtures/my-agents.html?browser&computers=none');
  await page.getByRole('main').getByRole('button', invite).click();
  const dialog = page.getByRole('dialog', { name: 'Bring an agent' });
  await expect(dialog.getByRole('radio', { name: /^Network/u })).toHaveAttribute(
    'aria-checked',
    'true',
  );
  // 换到命令行：只说要装，这个系统没有下载。
  await dialog.getByRole('radio', { name: /^Command line/u }).click();
  await expect(
    dialog.getByText(/The agent’s computer needs Agent Room installed and signed in/u),
  ).toBeVisible();
  await expect(dialog.getByRole('link', { name: 'Download Agent Room' })).toHaveCount(0);
  expect(failures).toEqual([]);
  await context.close();
});
