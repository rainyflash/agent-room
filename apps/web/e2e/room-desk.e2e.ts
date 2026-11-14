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
