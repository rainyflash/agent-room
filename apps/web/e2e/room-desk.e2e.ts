import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openRoomSettings } from './support/workspace-navigation';

// 房间页的对话版面（specs/redesign/room-page.md）。1b 起它是默认的，
// 人在顶栏换成场景以后这台设备记着。
const room = '/e2e/fixtures/lobby-scene.html?private&history&agents=8';

function layoutOption(page: Page, name: 'Conversation' | 'Scene') {
  return page
    .getByRole('radiogroup', { name: 'Room layout', exact: true })
    .getByRole('radio', { name, exact: true });
}

/** 主房间里来一条别人说的话。 */
async function receiveInMainRoom(page: Page, text: string): Promise<void> {
  await page.evaluate(
    (text) =>
      (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({ agentIndex: 2, text }),
    text,
  );
}

/** 换配色以后等颜色过渡走完：走到一半时按钮上的字和底都还在半路上，量出来的对比度不作数。 */
async function useColorScheme(page: Page, colorScheme: 'dark' | 'light'): Promise<void> {
  await page.emulateMedia({ colorScheme });
  await page.evaluate(async () => {
    const transitions = document
      .getAnimations()
      .filter((animation) => animation instanceof CSSTransition);
    await Promise.all(transitions.map((transition) => transition.finished.catch(() => null)));
  });
}

async function expectAccessible(page: Page): Promise<void> {
  const accessibility = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(accessibility.violations).toEqual([]);
}

test('默认是对话版面，打开就能看能写；换成场景以后这台设备记着', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(room);
  const canvas = page.locator('.lobby-scene__canvas');
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  // 地址里没写版面，这台设备也没换过：对话版面，不用点任何东西。
  await expect(input).toBeInViewport();
  await expect(canvas).toHaveCount(0);
  await expect(layoutOption(page, 'Conversation')).toHaveAttribute('aria-checked', 'true');
  await expect(page).not.toHaveURL(/layout=/u);
  // 三栏：主房间和私聊、对话、Agent。
  await expect(
    page.getByRole('navigation', { name: 'Main room and direct messages', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('region', { name: 'Conversation and shared resources', exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole('complementary', { name: 'Agents in this room', exact: true }),
  ).toBeVisible();
  await expect(page.getByText('Review milestone 80', { exact: false }).first()).toBeVisible();
  await expectAccessible(page);

  await layoutOption(page, 'Scene').click();
  await expect(canvas).toBeVisible();
  await expect(input).toBeHidden();
  await expect(page).toHaveURL(/layout=scene/u);
  // 顶栏换了一条，焦点接着落在新的切换上。
  await expect(layoutOption(page, 'Scene')).toBeFocused();

  await page.reload();
  await expect(canvas).toBeVisible();
  // 地址里没写版面：照这台设备记着的。
  await page.goto(room);
  await expect(canvas).toBeVisible();
  await expect(layoutOption(page, 'Scene')).toHaveAttribute('aria-checked', 'true');

  // 键盘也换得回去。
  await layoutOption(page, 'Scene').press('ArrowLeft');
  await expect(input).toBeInViewport();
  await expect(canvas).toHaveCount(0);
  await expect(page).toHaveURL(/layout=conversation/u);
  await expect(layoutOption(page, 'Conversation')).toBeFocused();

  // 地址里写了版面就照地址，不管设备记着什么。
  await page.goto(`${room}&layout=scene`);
  await expect(canvas).toBeVisible();
  expect(failures).toEqual([]);
});

test('对话版面：Agent 详情盖在右栏上，私聊在主栏里，顶栏的动作都能用', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${room}&layout=conversation`);
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await input.fill('A draft that should stay.');

  const agents = page.getByRole('complementary', { name: 'Agents in this room', exact: true });
  await agents.getByRole('button', { name: /Build Agent 003/u }).click();
  const details = page.locator('.agent-inspector');
  await expect(details).toContainText('Build Agent 003');
  await expect(details).toHaveCSS('opacity', '1');
  // 对话没有被收起来，草稿也在。
  await expect(input).toBeVisible();
  await expect(input).toHaveValue('A draft that should stay.');
  const covered = await page.evaluate(() => {
    const box = (selector: string) => document.querySelector(selector)?.getBoundingClientRect();
    const inspector = box('.agent-inspector');
    const side = box('.room-desk__side');
    const main = box('.room-desk__main');
    if (inspector === undefined || side === undefined || main === undefined) return null;
    return {
      overMain: inspector.left < main.right - 1,
      overSide: Math.abs(inspector.left - side.left) <= 1 && inspector.right >= side.right - 1,
    };
  });
  expect(covered).toEqual({ overMain: false, overSide: true });
  // 详情盖着的时候底下的表格碰不到：键盘和读屏都不会走进去。
  await expect(agents.getByRole('table')).toHaveCount(0);
  await expect(agents.locator('.room-roster')).toBeHidden();
  // 这里关掉就是回到表格，所以写的是“回到列表”，没有另外的叉；焦点一开始就在它上面。
  const back = details.getByRole('button', { name: 'Back to the list', exact: true });
  await expect(back).toBeFocused();
  await expect(page.getByRole('button', { name: 'Close Agent details', exact: true })).toHaveCount(
    0,
  );
  await expectAccessible(page);
  await back.click();
  await expect(details).toHaveCount(0);
  // 焦点回到右栏里刚才那一行。
  await expect(agents.getByRole('button', { name: /Build Agent 003/u })).toBeFocused();

  // 私聊显示在主栏里，“主房间”一点就回来。
  const places = page.getByRole('navigation', { name: 'Main room and direct messages' });
  const mainRoom = places.getByRole('button', { name: 'Main room', exact: true });
  await expect(mainRoom).toHaveAttribute('aria-current', 'page');
  await page.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
  await expect(page.getByRole('heading', { name: 'Build Agent 002', exact: true })).toBeVisible();
  await expect(mainRoom).not.toHaveAttribute('aria-current', 'page');
  await mainRoom.click();
  await expect(mainRoom).toHaveAttribute('aria-current', 'page');
  await expect(input).toHaveValue('A draft that should stay.');

  // 顶栏：接入 Agent 是唯一的主按钮，房间设置在齿轮里。
  await expect(page.locator('.room-desk__top .ar-button--primary')).toHaveCount(1);
  await page.getByRole('button', { name: 'Bring an agent', exact: true }).click();
  const invite = page.getByRole('dialog', { name: 'Bring an agent' });
  await expect(invite).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(invite).toBeHidden();
  const settings = await openRoomSettings(page);
  await expect(settings).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(settings).toBeHidden();
  await expect(page.getByRole('link', { name: 'Back to rooms', exact: true })).toBeVisible();
  expect(failures).toEqual([]);
});

test('人不在主房间的对话里时来了新消息：回去的地方标着条数，回去就没了', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(room);
  const log = page.locator('.workspace-room-content').getByRole('log');
  const unread = page.locator('.room-unread');
  // 在主房间的对话里：来了就看到了，不标。
  await receiveInMainRoom(page, 'Seen right away.');
  await expect(log).toContainText('Seen right away.');
  await expect(unread).toHaveCount(0);

  // 在私聊里：左栏的“主房间”上标着。名字不变，条数读屏读得到。
  const mainRoom = page.getByRole('button', { name: 'Main room', exact: true });
  await page.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
  await expect(page.getByRole('heading', { name: 'Build Agent 002', exact: true })).toBeVisible();
  await receiveInMainRoom(page, 'Arrived while in a direct message.');
  await expect(mainRoom).toHaveAccessibleDescription('1 new message');
  await expect(mainRoom.locator('.room-unread')).toContainText('1');
  await expectAccessible(page);
  await mainRoom.click();
  await expect(log).toContainText('Arrived while in a direct message.');
  await expect(unread).toHaveCount(0);

  // 在看资料：标在“对话”那个标签上。
  await page.getByRole('tab', { name: 'Resources', exact: true }).click();
  await receiveInMainRoom(page, 'Arrived while reading resources.');
  await receiveInMainRoom(page, 'And one more.');
  const conversation = page.getByRole('tab', { name: 'Conversation', exact: true });
  await expect(conversation).toHaveAccessibleDescription('2 new messages');
  await expectAccessible(page);
  await conversation.click();
  await expect(log).toContainText('And one more.');
  await expect(unread).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('对话版面窄的时候：两栏收起来，Agent 从顶栏打开，私聊里留着回主房间的路', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1000, height: 760 });
  await page.goto(`${room}&layout=conversation`);
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await expect(input).toBeInViewport();
  await expect(
    page.getByRole('navigation', { name: 'Main room and direct messages' }),
  ).toBeHidden();
  await expect(
    page.getByRole('complementary', { name: 'Agents in this room', exact: true }),
  ).toBeHidden();
  await expectNoHorizontalOverflow(page);

  await page.getByRole('button', { name: 'View agents', exact: true }).click();
  const drawer = page.getByRole('dialog', { name: 'Agents in this room', exact: true });
  // 抽屉窄：表格只留名字和状态两列。
  await expect(drawer.getByRole('columnheader')).toHaveText(['Name', 'Status']);
  // 点一行：抽屉收起来，详情是从右边出来的一层，带着叉；关掉以后焦点回到顶栏打开抽屉的按钮。
  await drawer.getByRole('button', { name: /^Build Agent 003/u }).click();
  await expect(drawer).toBeHidden();
  const details = page.locator('.agent-inspector');
  await expect(details).toContainText('Build Agent 003');
  await expect(details.getByRole('button', { name: 'Back to the list', exact: true })).toHaveCount(
    0,
  );
  await details.getByRole('button', { name: 'Close Agent details', exact: true }).click();
  await expect(details).toHaveCount(0);
  const openAgents = page.getByRole('button', { name: 'View agents', exact: true });
  await expect(openAgents).toBeFocused();
  await openAgents.click();
  await drawer.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
  await expect(drawer).toBeHidden();
  await expect(page.getByRole('heading', { name: 'Build Agent 002', exact: true })).toBeVisible();
  // 在私聊里时主房间来了新消息：回去的那一条上标着。
  const back = page.getByRole('button', { name: 'Back to the main room', exact: true });
  await receiveInMainRoom(page, 'Arrived while in a direct message.');
  await expect(back).toHaveAccessibleDescription('1 new message');
  await expect(back.locator('.room-unread')).toBeVisible();
  await back.click();
  await expect(page.getByRole('heading', { name: 'Build Agent 002', exact: true })).toBeHidden();
  await expect(page.getByText('Review milestone 80', { exact: false }).first()).toBeVisible();
  await expectAccessible(page);
  expect(failures).toEqual([]);
});

test('对话版面在手机上：不横向滚动，输入框贴在底下，顶栏用一个按钮换版面', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(room);
  const canvas = page.locator('.lobby-scene__canvas');
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await expect(input).toBeInViewport();
  await expect(canvas).toHaveCount(0);
  // 手机上两段的切换换成一个按钮。
  await expect(page.getByRole('radiogroup', { name: 'Room layout', exact: true })).toBeHidden();
  await expectNoHorizontalOverflow(page);
  const gap = await page.evaluate(() => {
    const composer = document.querySelector('.room-desk__main .conversation-panel__composer');
    if (composer === null) return null;
    return Math.round(window.innerHeight - composer.getBoundingClientRect().bottom);
  });
  expect(gap).not.toBeNull();
  expect(gap).toBeGreaterThanOrEqual(0);
  expect(gap).toBeLessThanOrEqual(24);
  // 顶栏的动作一个不少，放不下的那一行自己左右滑。
  await expect(page.getByRole('button', { name: 'Bring an agent', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Room settings', exact: true })).toBeVisible();
  await expectAccessible(page);

  await page.getByRole('button', { name: 'Switch to the scene layout', exact: true }).click();
  await expect(canvas).toBeVisible();
  await expect(input).toBeHidden();
  await page
    .getByRole('button', { name: 'Switch to the conversation layout', exact: true })
    .click();
  await expect(input).toBeInViewport();
  await expect(canvas).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('认得出的记号：主房间前面是 #，在等消息的头像带绿点，找你的那一条整行铺一层浅黄', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(room);
  await expect(page.getByText('Review milestone 80', { exact: false }).first()).toBeVisible();

  const places = page.getByRole('navigation', {
    name: 'Main room and direct messages',
    exact: true,
  });
  await expect(places.locator('.ar-room-glyph')).toHaveText('#');

  // 表格里只有在等消息的那一个头像右下角有点；状态一列照旧用字写着，点不单独读。
  const roster = page.getByRole('complementary', { name: 'Agents in this room', exact: true });
  const dotted = roster.getByRole('row').filter({ has: page.locator('.ar-presence-dot--waiting') });
  await expect(dotted).toHaveCount(1);
  await expect(dotted.locator('.agent-table__state')).toHaveText('Waiting');
  await expect(dotted.locator('.ar-presence-dot')).toHaveAttribute('aria-hidden', 'true');
  await expect(roster.locator('tbody .ar-avatar')).toHaveCount(8);

  // 对话里：Agent 是机器人垫着浅底；点了你名字的那一条整行有底色，别的没有。
  const mentioned = page.locator('.conversation-message[data-mentioned="true"]').last();
  await expect(mentioned).toContainText('Review milestone 79');
  await expect(mentioned.locator('.ar-avatar svg.agent-portrait')).toBeVisible();
  await expect(mentioned).toHaveCSS('background-color', 'rgb(255, 248, 225)');
  const plain = page.locator('.conversation-message:not([data-mentioned])').last();
  await expect(plain).toContainText('Review milestone 80');
  await expect(plain).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
  await expectAccessible(page);

  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(mentioned).toHaveCSS('background-color', 'rgb(42, 37, 18)');
  await expectAccessible(page);
  expect(failures).toEqual([]);
});

test('Agent 表格：谁的、什么状态一眼看到，叫得醒的标出来，点一行看详情；“人”列出成员', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  // 夹具里第 1、5 个 Agent 是你的，第 5 个开着后台回复；第 3 个在等消息，第 7 个离线。
  await page.goto(`${room}&owners&wakes`);
  const side = page.getByRole('complementary', { name: 'Agents in this room', exact: true });
  const row = (name: string) =>
    side
      .getByRole('row')
      .filter({ has: page.getByRole('button', { name: new RegExp(`^${name}`, 'u') }) });

  // 顶栏上的个数和标签上的是同一个数。
  const meta = page.locator('.room-desk__meta');
  await expect(meta).toContainText('2 people');
  await expect(meta).toContainText('8 agents');
  const tabs = side.getByRole('tablist', { name: 'Who is in this room', exact: true });
  const agentsTab = tabs.getByRole('tab', { name: 'Agents 8', exact: true });
  const peopleTab = tabs.getByRole('tab', { name: 'People 2', exact: true });
  await expect(agentsTab).toHaveAttribute('aria-selected', 'true');
  await expect(side.locator('.room-roster__summary > span')).toHaveText([
    '1 waiting',
    '1 wakes on mention',
    '1 offline',
  ]);
  await expect(side.getByRole('columnheader')).toHaveText(['Name', 'Owner', 'Status']);
  // 在等的在最前，然后是叫得醒的；离线的在最后。
  await expect(side.locator('tbody .agent-table__label')).toHaveText([
    'Build Agent 003',
    'Build Agent 005',
    'Build Agent 001',
    'Build Agent 002',
    'Build Agent 004',
    'Build Agent 006',
    'Build Agent 008',
    'Build Agent 007',
  ]);
  await expect(row('Build Agent 003').getByRole('cell')).toHaveText(['Network', 'Waiting']);
  await expect(row('Build Agent 005').getByRole('cell')).toHaveText(['You', 'Wakes on mention']);
  await expect(row('Build Agent 005').locator('.ar-presence-dot--wakes')).toHaveCount(1);
  await expect(row('Build Agent 002').getByRole('cell')).toHaveText(['Grace', 'Online']);
  await expect(row('Build Agent 007').getByRole('cell')).toHaveText([
    'Network',
    /^Offline · \d+ minutes ago$/u,
  ]);
  await expect(row('Build Agent 007')).toHaveAttribute('data-state', 'offline');
  // 右栏里滚出去的东西不把整页撑高。
  expect(await page.locator('main').evaluate((main) => main.scrollHeight - main.clientHeight)).toBe(
    0,
  );
  await expectAccessible(page);

  // 键盘：上下键在行之间走，回车看详情；详情上的头像带着同一个“叫得醒”的圈。
  await side.getByRole('button', { name: /^Build Agent 003/u }).focus();
  await page.keyboard.press('ArrowDown');
  const mine = side.getByRole('button', { name: /^Build Agent 005/u });
  await expect(mine).toBeFocused();
  await page.keyboard.press('Enter');
  const details = page.locator('.agent-inspector');
  await expect(details).toContainText('Build Agent 005');
  await expect(details.locator('.ar-presence-dot--wakes')).toHaveCount(1);
  const back = details.getByRole('button', { name: 'Back to the list', exact: true });
  await expect(back).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(details).toHaveCount(0);
  await expect(mine).toBeFocused();

  // 搜索只筛表格里的行。
  const search = side.getByRole('searchbox');
  await search.fill('agent 00');
  await expect(side.locator('tbody tr')).toHaveCount(8);
  await search.fill('agent 007');
  await expect(side.locator('tbody .agent-table__label')).toHaveText(['Build Agent 007']);
  await search.fill('nobody by that name');
  await expect(side.getByText('No agent matches that.', { exact: true })).toBeVisible();
  await search.fill('');
  await expect(side.locator('tbody tr')).toHaveCount(8);

  // “人”：成员、身份、各自带了几个 Agent；你自己那一行标着。
  await peopleTab.click();
  await expect(peopleTab).toHaveAttribute('aria-selected', 'true');
  await expect(side.getByRole('columnheader')).toHaveText(['Name', 'Role', 'Agents']);
  await expect(side.getByRole('rowheader')).toHaveText([/Ada you$/u, /Grace$/u]);
  const person = (name: string) => side.getByRole('row').filter({ hasText: name });
  await expect(person('Ada').getByRole('cell')).toHaveText(['Owner', '2']);
  await expect(person('Grace').getByRole('cell')).toHaveText(['', '2']);
  await expect(side.locator('.room-roster__note')).toHaveCount(0);
  await expectAccessible(page);
  // 标签之间左右键能走。
  await peopleTab.press('ArrowLeft');
  await expect(agentsTab).toBeFocused();
  await expect(agentsTab).toHaveAttribute('aria-selected', 'true');

  await useColorScheme(page, 'dark');
  await expectAccessible(page);
  await useColorScheme(page, 'light');

  // 右栏窄的那一段：只留名字和状态两列，“谁的”写在名字下面。
  await page.setViewportSize({ width: 1280, height: 800 });
  await expect(side.getByRole('columnheader')).toHaveText(['Name', 'Status']);
  await expect(row('Build Agent 005').locator('.agent-table__owner-line')).toBeVisible();
  await expect(row('Build Agent 005').locator('.agent-table__owner-line')).toHaveText('You');
  await expect(row('Build Agent 005').getByRole('cell')).toHaveText(['Wakes on mention']);
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  expect(failures).toEqual([]);
});

test('展开成整页：Agent 表格占满主栏和右栏，多三列，能筛；详情在旁边，点房间名回对话', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  // 夹具里第 1、5 个 Agent 是你的（第 5 个开着后台回复），第 3、7 个是 Grace 放行的网络 Agent，
  // 第 4、8 个是没人放行的；`&cards` 里它们依次在等消息、下次运行时读、离线，名片是三天前写的。
  await page.goto(`${room}&owners&wakes&cards`);
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await input.fill('A draft that should stay.');
  const side = page.getByRole('complementary', { name: 'Agents in this room', exact: true });
  const expand = side.getByRole('button', { name: 'Open as a full page', exact: true });
  await expand.click();

  // 这一页占着主栏和右栏：对话和右栏都不在了，焦点落在这一页的标题上。
  const agents = page.getByRole('region', { name: 'Agents in this room', exact: true });
  await expect(agents.getByRole('heading', { level: 2 })).toBeFocused();
  await expect(side).toHaveCount(0);
  await expect(input).toHaveCount(0);
  expect(new URL(page.url()).searchParams.get('view')).toBe('agents');
  // 顶栏：房间名 / Agent。左栏的“主房间”不再标成当前这一页。
  const crumbs = page.getByRole('heading', { level: 1 });
  await expect(crumbs.locator('[aria-current="page"]')).toHaveText('Agents');
  const backToRoom = crumbs.getByRole('button', { name: 'Back to Builders Exchange', exact: true });
  await expect(backToRoom).toHaveText('Builders Exchange');
  const mainRoom = page
    .getByRole('navigation', { name: 'Main room and direct messages' })
    .getByRole('button', { name: 'Main room', exact: true });
  await expect(mainRoom).not.toHaveAttribute('aria-current', 'page');

  await expect(agents.locator('.agent-page__summary > span')).toHaveText([
    '3 waiting',
    '1 wakes on mention',
    '2 offline',
  ]);
  await expect(agents.getByRole('columnheader')).toHaveText([
    'Name',
    'Owner',
    'Status',
    'How it joined',
    'Joined',
    'Your tags',
  ]);
  const labels = agents.locator('tbody .agent-table__label');
  await expect(labels).toHaveText([
    'Build Agent 001',
    'Build Agent 004',
    'Build Agent 007',
    'Build Agent 005',
    'Build Agent 002',
    'Build Agent 008',
    // 最后是离线的两个。它们差不多同时离线（夹具里各自取的“20 分钟前”），谁先谁后差在毫秒上。
    /^Build Agent 00[36]$/u,
    /^Build Agent 00[36]$/u,
  ]);
  const row = (name: string) =>
    agents
      .getByRole('row')
      .filter({ has: page.getByRole('button', { name: new RegExp(`^${name}`, 'u') }) });
  // 进来的时间是名片的时间：三天前，只写月和日（跨年时带上年）。
  const joined = /^[A-Z][a-z]{2} \d{1,2}(, \d{4})?$/u;
  // 浏览器里问不到这台电脑上有谁，自己的 Agent 也只说是从电脑上接进来的。
  await expect(row('Build Agent 001').getByRole('cell')).toHaveText([
    'You',
    'Waiting',
    'A computer',
    joined,
    '',
  ]);
  await expect(row('Build Agent 004').getByRole('cell')).toHaveText([
    'Network',
    'Waiting',
    'Network',
    joined,
    '',
  ]);
  await expect(row('Build Agent 007').getByRole('cell')).toHaveText([
    'Network',
    'Waiting',
    'Network · Let in by Grace',
    joined,
    '',
  ]);
  await expect(row('Build Agent 005').getByRole('cell')).toHaveText([
    'You',
    'Wakes on mention',
    'A computer',
    joined,
    '',
  ]);
  await expect(row('Build Agent 006').getByRole('cell')).toHaveText([
    'Grace',
    /^Offline · \d+ minutes ago$/u,
    'A computer',
    joined,
    '',
  ]);
  // 黑色的按钮还是只有顶栏那一个；这一页自己滚，不把整页撑高。
  await expect(page.locator('.ar-button--primary').filter({ visible: true })).toHaveCount(1);
  expect(await page.locator('main').evaluate((main) => main.scrollHeight - main.clientHeight)).toBe(
    0,
  );
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  await useColorScheme(page, 'dark');
  await expectAccessible(page);
  await useColorScheme(page, 'light');

  // 筛：谁的、状态，几样一起用；你还没做过记号，没有“你的标签”那一组。
  const owner = agents.getByRole('group', { name: 'Owner', exact: true });
  const state = agents.getByRole('group', { name: 'Status', exact: true });
  await expect(agents.getByRole('group', { name: 'Your tags', exact: true })).toHaveCount(0);
  await expect(state.getByRole('button')).toHaveText([
    'All',
    'Waiting',
    'Wakes on mention',
    'Offline',
  ]);
  await owner.getByRole('button', { name: 'Mine', exact: true }).click();
  await expect(labels).toHaveText(['Build Agent 001', 'Build Agent 005']);
  await expect(owner.getByRole('button', { name: 'Mine', exact: true })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await state.getByRole('button', { name: 'Waiting', exact: true }).click();
  await expect(labels).toHaveText(['Build Agent 001']);
  await owner.getByRole('button', { name: 'Network agents', exact: true }).click();
  await expect(labels).toHaveText(['Build Agent 004', 'Build Agent 007']);
  await state.getByRole('button', { name: 'Wakes on mention', exact: true }).click();
  await expect(agents.getByText('No agent matches that.', { exact: true })).toBeVisible();
  await owner.getByRole('button', { name: 'Everyone’s', exact: true }).click();
  await expect(labels).toHaveText(['Build Agent 005']);
  await state.getByRole('button', { name: 'All', exact: true }).click();
  const search = agents.getByRole('searchbox');
  await search.fill('agent 007');
  await expect(labels).toHaveText(['Build Agent 007']);
  await search.fill('');
  await expect(labels).toHaveCount(8);

  // 键盘：上下键在行之间走，回车看详情。详情占着右栏那一格、带着叉，表格让出来、照剩下的宽度收列。
  await agents.getByRole('button', { name: /^Build Agent 001/u }).focus();
  await page.keyboard.press('ArrowDown');
  const second = agents.getByRole('button', { name: /^Build Agent 004/u });
  await expect(second).toBeFocused();
  await page.keyboard.press('Enter');
  const details = page.locator('.agent-inspector');
  await expect(details).toContainText('Build Agent 004');
  await expect(details).toHaveCSS('opacity', '1');
  const close = details.getByRole('button', { name: 'Close Agent details', exact: true });
  await expect(close).toBeFocused();
  await expect(details.getByRole('button', { name: 'Back to the list', exact: true })).toHaveCount(
    0,
  );
  await expect(row('Build Agent 004')).toHaveAttribute('data-selected', 'true');
  await expect(agents.getByRole('columnheader')).toHaveText([
    'Name',
    'Owner',
    'Status',
    'How it joined',
  ]);
  const beside = await page.evaluate(() => {
    const box = (selector: string) => document.querySelector(selector)?.getBoundingClientRect();
    const inspector = box('.agent-inspector');
    const list = box('.room-desk__page');
    return inspector === undefined || list === undefined ? null : inspector.left >= list.right - 1;
  });
  expect(beside).toBe(true);
  await expectNoHorizontalOverflow(page);
  // 在详情里收藏、打标签：关掉以后焦点回到那一行，记号写在最后一列，上面多出能按它们筛的一组。
  await details.getByRole('button', { name: 'Add to favorites', exact: true }).click();
  await details.getByRole('textbox', { name: 'Project tags', exact: true }).fill('release');
  await details.getByRole('button', { name: 'Save tags', exact: true }).click();
  await close.click();
  await expect(details).toHaveCount(0);
  await expect(second).toBeFocused();
  await expect(agents.getByRole('columnheader')).toHaveCount(6);
  await expect(row('Build Agent 004').locator('.agent-table__marks > span')).toHaveText([
    'Favorite',
    'release',
  ]);
  const marks = agents.getByRole('group', { name: 'Your tags', exact: true });
  await expect(marks.getByRole('button')).toHaveText(['All', 'Favorites', 'release']);
  await marks.getByRole('button', { name: 'release', exact: true }).click();
  await expect(labels).toHaveText(['Build Agent 004']);
  await marks.getByRole('button', { name: 'All', exact: true }).click();
  await expect(labels).toHaveCount(8);

  // 这时主房间来了新消息：标在回去的两个地方。点房间名回对话，草稿还在，焦点回到“展开成整页”。
  await receiveInMainRoom(page, 'Arrived while looking at the table.');
  await expect(backToRoom).toHaveAccessibleDescription('1 new message');
  await expect(mainRoom).toHaveAccessibleDescription('1 new message');
  await backToRoom.click();
  await expect(agents).toHaveCount(0);
  await expect(input).toHaveValue('A draft that should stay.');
  await expect(page.locator('.workspace-room-content').getByRole('log')).toContainText(
    'Arrived while looking at the table.',
  );
  await expect(page.locator('.room-unread')).toHaveCount(0);
  await expect(expand).toBeFocused();
  await expect(mainRoom).toHaveAttribute('aria-current', 'page');
  expect(new URL(page.url()).searchParams.get('view')).toBe('conversation');
  // 左栏的“主房间”也是回去的路。
  await expand.click();
  await expect(agents).toBeVisible();
  await mainRoom.click();
  await expect(agents).toHaveCount(0);
  await expect(input).toBeVisible();
  expect(failures).toEqual([]);
});

test('展开的 Agent 表格窄的时候：从抽屉里展开，列按宽度收，详情是从右边出来的一层', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1000, height: 760 });
  await page.goto(`${room}&owners&wakes&cards`);
  const openAgents = page.getByRole('button', { name: 'View agents', exact: true });
  await openAgents.click();
  const drawer = page.getByRole('dialog', { name: 'Agents in this room', exact: true });
  await drawer.getByRole('button', { name: 'Open as a full page', exact: true }).click();
  await expect(drawer).toBeHidden();
  const agents = page.getByRole('region', { name: 'Agents in this room', exact: true });
  await expect(agents.getByRole('heading', { level: 2 })).toBeFocused();
  // 已经在这一页上了，顶栏不再留打开抽屉的按钮。
  await expect(openAgents).toBeHidden();
  const headers = agents.getByRole('columnheader');
  await expect(headers).toHaveCount(6);
  await expectNoHorizontalOverflow(page);

  // 没有右栏：详情是盖在右边的一层，带着叉；关掉以后焦点回到那一行。
  const name = agents.getByRole('button', { name: /^Build Agent 004/u });
  await name.click();
  const details = page.locator('.agent-inspector');
  await expect(details).toContainText('Build Agent 004');
  await expect(headers).toHaveCount(6);
  await details.getByRole('button', { name: 'Close Agent details', exact: true }).click();
  await expect(details).toHaveCount(0);
  await expect(name).toBeFocused();

  // 越窄列越少：先收标签和进来的时间，再是怎么来的；最窄只留名字和状态，“谁的”写在名字下面。
  await page.setViewportSize({ width: 820, height: 760 });
  await expect(headers).toHaveText(['Name', 'Owner', 'Status', 'How it joined']);
  await page.setViewportSize({ width: 600, height: 760 });
  await expect(headers).toHaveText(['Name', 'Owner', 'Status']);
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(headers).toHaveText(['Name', 'Status']);
  const mine = agents
    .getByRole('row')
    .filter({ has: page.getByRole('button', { name: /^Build Agent 001/u }) });
  await expect(mine.locator('.agent-table__owner-line')).toBeVisible();
  await expect(mine.locator('.agent-table__owner-line')).toHaveText('You');
  await expect(mine.getByRole('cell')).toHaveText(['Waiting']);
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);

  // 点房间名回对话：焦点回到顶栏打开 Agent 的按钮。
  await page.getByRole('button', { name: 'Back to Builders Exchange', exact: true }).click();
  await expect(agents).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeVisible();
  await expect(openAgents).toBeFocused();
  expect(failures).toEqual([]);
});

test('展开的 Agent 表格：地址里写着就直接打开，人多时一次列 100 个；场景里没有这一页', async ({
  page,
}) => {
  // 240 个 Agent 的房间在几个用例一起跑的时候画得慢（本机 WebKit 整套跑时超过 30 秒）。
  test.setTimeout(60_000);
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-scene.html?private&agents=240&view=agents');
  const agents = page.getByRole('region', { name: 'Agents in this room', exact: true });
  const rows = agents.locator('tbody tr');
  await expect(rows).toHaveCount(100);
  await expect(agents.getByText('Showing 100 of 240', { exact: true })).toBeVisible();
  await agents.getByRole('button', { name: 'Show 100 more', exact: true }).click();
  await expect(rows).toHaveCount(200);
  await agents.getByRole('button', { name: 'Show 40 more', exact: true }).click();
  await expect(rows).toHaveCount(240);
  await expect(agents.getByText('Showing 240 of 240', { exact: true })).toBeVisible();
  await expect(agents.getByRole('button', { name: /^Show \d+ more$/u })).toHaveCount(0);
  // 这一页自己滚；整页不跟着长高。
  expect(await agents.evaluate((list) => list.scrollHeight > list.clientHeight)).toBe(true);
  expect(await page.locator('main').evaluate((main) => main.scrollHeight - main.clientHeight)).toBe(
    0,
  );
  // 搜一下从头列起；不到 100 个时下面什么都不写。
  await agents.getByRole('searchbox').fill('agent 23');
  await expect(rows).toHaveCount(10);
  await expect(agents.getByText(/^Showing/u)).toHaveCount(0);

  // 换成场景：那里没有这一页，回到画面本身。
  await layoutOption(page, 'Scene').click();
  await expect(agents).toHaveCount(0);
  await expect(page.locator('.room-scene')).toBeVisible();
  expect(new URL(page.url()).searchParams.get('view')).toBeNull();
  expect(failures).toEqual([]);
});

test('公共大厅没有成员名单：“人”列的是说过话的人和你自己，并且照实说', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-scene.html?history&agents=8&owners');
  const side = page.getByRole('complementary', { name: 'Agents in this room', exact: true });
  const meta = page.locator('.room-desk__meta');
  await expect(meta).toContainText('1 person');
  await expect(meta).toContainText('8 agents');
  await side.getByRole('tab', { name: 'People 1', exact: true }).click();
  await expect(side.getByRole('rowheader')).toHaveText([/Fixture operator you$/u]);
  // 大厅里没有房主、管理员；Agent 数照服务器说的主人算。
  await expect(side.getByRole('row').nth(1).getByRole('cell')).toHaveText(['', '2']);
  await expect(side.locator('.room-roster__note')).toHaveText(
    'The public lobby has no member list. These are the people who have spoken, and you.',
  );
  await expectAccessible(page);
  expect(failures).toEqual([]);
});

test('还没有 Agent 的房间：右栏是一句话加“接入 Agent”，对话照常能用', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-scene.html?private&agents=0');
  const side = page.getByRole('complementary', { name: 'Agents in this room', exact: true });
  await expect(side.getByRole('tab', { name: 'Agents 0', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  await expect(page.locator('.room-desk__meta')).toContainText('0 agents');
  await expect(side.getByText('No agents here yet', { exact: true })).toBeVisible();
  await expect(side.getByRole('table')).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeEnabled();
  // 黑色的按钮还是只有顶栏那一个，右栏这个是灰的。
  await expect(page.locator('.ar-button--primary').filter({ visible: true })).toHaveCount(1);
  await expectAccessible(page);
  await side.getByRole('button', { name: 'Bring an agent', exact: true }).click();
  const invite = page.getByRole('dialog', { name: 'Bring an agent' });
  await expect(invite).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(invite).toBeHidden();
  expect(failures).toEqual([]);
});
