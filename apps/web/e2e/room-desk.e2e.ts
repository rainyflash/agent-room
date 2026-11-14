import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import {
  backToConversationButton,
  openMainRoom,
  openResources,
  openRoomSettings,
  placesButton,
} from './support/workspace-navigation';

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

/**
 * 换配色以后等颜色过渡走完：走到一半时按钮上的字和底都还在半路上，量出来的对比度不作数。
 * 等到一个过渡都不剩为止：头一批走完时可能又起了新的（颜色是继承来的、自己又带过渡的元素，
 * 上一层每走一帧它就重新起一次）。换了配色接着查对比度的用例都用它，别直接 `emulateMedia`。
 */
async function useColorScheme(page: Page, colorScheme: 'dark' | 'light'): Promise<void> {
  await page.emulateMedia({ colorScheme });
  await page.evaluate(async () => {
    const running = () =>
      document.getAnimations().filter((animation) => animation instanceof CSSTransition);
    for (let transitions = running(), round = 0; transitions.length > 0; transitions = running()) {
      if ((round += 1) > 100) throw new Error('换配色以后颜色过渡一直没走完。');
      await Promise.all(transitions.map((transition) => transition.finished.catch(() => null)));
    }
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

  // 在看资料：标在“回到对话”上。点“资料”换过去，焦点落在“回到对话”上；回来落回“资料”。
  await openResources(page);
  const conversation = backToConversationButton(page);
  await expect(conversation).toBeFocused();
  await receiveInMainRoom(page, 'Arrived while reading resources.');
  await receiveInMainRoom(page, 'And one more.');
  await expect(conversation).toHaveAccessibleDescription('2 new messages');
  await expectAccessible(page);
  await conversation.click();
  await expect(log).toContainText('And one more.');
  await expect(unread).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Resources', exact: true })).toBeFocused();
  expect(failures).toEqual([]);
});

test('对话版面窄的时候：右栏收成顶栏上的按钮，左栏收进房间名旁边的下拉，私聊里留着回主房间的路', async ({
  page,
}) => {
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
  // 抽屉里只有 Agent 和人：私聊在房间名旁边的下拉里。
  await openAgents.click();
  await expect(drawer.getByRole('button', { name: /^Open conversation with/u })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(drawer).toBeHidden();

  // 下拉上写着现在在哪儿，点开是主房间和私聊，贴在它下面。
  const places = placesButton(page);
  const nowIn = (place: string) => `Go to the main room or a direct message (now in ${place})`;
  await expect(places).toHaveAccessibleName(nowIn('Main room'));
  await expect(places).toHaveAttribute('aria-expanded', 'false');
  await places.click();
  await expect(places).toHaveAttribute('aria-expanded', 'true');
  const menu = page.getByRole('dialog', { name: 'Main room and direct messages', exact: true });
  const mainRoom = menu.getByRole('button', { name: 'Main room', exact: true });
  await expect(mainRoom).toHaveAttribute('aria-current', 'page');
  await expect(mainRoom).toBeFocused();
  const placesBox = await places.boundingBox();
  const menuBox = await menu.boundingBox();
  if (placesBox === null || menuBox === null) throw new Error('没有量到下拉的位置。');
  expect(menuBox.y).toBeGreaterThanOrEqual(placesBox.y + placesBox.height);
  expect(menuBox.y).toBeLessThanOrEqual(placesBox.y + placesBox.height + 12);
  expect(Math.abs(menuBox.x - placesBox.x)).toBeLessThanOrEqual(2);
  await expectAccessible(page);
  // Esc 收起来，焦点回到那个按钮上；点外面也收。
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(places).toBeFocused();
  await places.click();
  await expect(menu).toBeVisible();
  await page.mouse.click(700, 520);
  await expect(menu).toBeHidden();

  // 点一场私聊：下拉收起来，上面写的换成对面的名字。
  await places.click();
  await menu.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
  await expect(menu).toBeHidden();
  await expect(page.getByRole('heading', { name: 'Build Agent 002', exact: true })).toBeVisible();
  await expect(places).toHaveAccessibleName(nowIn('Build Agent 002'));
  // 在私聊里时主房间来了新消息：下拉上、主栏顶上回去的那一条上都标着。
  const back = page.getByRole('button', { name: 'Back to the main room', exact: true });
  await receiveInMainRoom(page, 'Arrived while in a direct message.');
  await expect(back).toHaveAccessibleDescription('1 new message');
  await expect(back.locator('.room-unread')).toBeVisible();
  await expect(places).toHaveAccessibleDescription('1 new message');
  await expect(places.locator('.room-unread')).toBeVisible();
  await places.click();
  await expect(mainRoom).not.toHaveAttribute('aria-current', 'page');
  await expect(mainRoom).toHaveAccessibleDescription('1 new message');
  await expectAccessible(page);
  await mainRoom.click();
  await expect(menu).toBeHidden();
  await expect(page.getByRole('heading', { name: 'Build Agent 002', exact: true })).toBeHidden();
  await expect(page.getByText('Review milestone 80', { exact: false }).first()).toBeVisible();
  await expect(places).toHaveAccessibleName(nowIn('Main room'));
  await expect(page.locator('.room-unread')).toHaveCount(0);

  // 窗口拉宽到左栏出来：开着的下拉跟着收起来，不留一层挡着页面。
  await places.click();
  await expect(menu).toBeVisible();
  await page.setViewportSize({ width: 1280, height: 760 });
  await expect(menu).toHaveCount(0);
  await expect(places).toBeHidden();
  await expect(
    page.getByRole('navigation', { name: 'Main room and direct messages' }),
  ).toBeVisible();
  await input.fill('Still usable.');
  await expect(input).toHaveValue('Still usable.');
  expect(failures).toEqual([]);
});

test('对话版面在手机上：顶栏只有一行，别的动作都在“更多”里；不横向滚动，输入框贴在底下', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(room);
  const canvas = page.locator('.lobby-scene__canvas');
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await expect(input).toBeInViewport();
  await expect(canvas).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  const gap = await page.evaluate(() => {
    const composer = document.querySelector('.room-desk__main .conversation-panel__composer');
    if (composer === null) return null;
    return Math.round(window.innerHeight - composer.getBoundingClientRect().bottom);
  });
  expect(gap).not.toBeNull();
  expect(gap).toBeGreaterThanOrEqual(0);
  expect(gap).toBeLessThanOrEqual(24);
  // 顶栏一行：回到房间列表、在哪儿（下面一行是房间名）、“Agent N”、“更多”。
  const top = page.locator('.room-desk__top');
  await expect.poll(() => topBarHeight(page)).toBeLessThanOrEqual(60);
  await expect(top.getByRole('link', { name: 'Back to rooms', exact: true })).toBeVisible();
  const places = placesButton(page);
  const roomName = top.getByRole('heading', { level: 1, name: 'Builders Exchange', exact: true });
  const placesBox = await places.boundingBox();
  const nameBox = await roomName.boundingBox();
  if (placesBox === null || nameBox === null) throw new Error('没有量到房间名。');
  expect(nameBox.y).toBeGreaterThanOrEqual(placesBox.y + placesBox.height - 2);
  const openAgents = top.getByRole('button', { name: 'View agents', exact: true });
  await expect(openAgents).toBeVisible();
  await expect(openAgents.locator('.room-desk__agents-word')).toBeHidden();
  const more = top.getByRole('button', { name: 'More', exact: true });
  await expect(more).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByRole('radiogroup', { name: 'Room layout', exact: true })).toBeHidden();
  for (const name of [
    'Bring an agent',
    'Invite people',
    'Room settings',
    'Switch to the scene layout',
  ])
    await expect(page.getByRole('button', { name, exact: true })).toBeHidden();
  await expectAccessible(page);

  // “更多”：从下面出来的一层，后面垫暗。
  await more.click();
  const menu = page.getByRole('dialog', { name: 'More', exact: true });
  await expect(menu.getByRole('button')).toHaveText([
    'Search this conversation',
    'Invite people',
    'Bring an agent',
    'Room settings',
    'Switch to the scene layout',
  ]);
  const menuBox = await menu.boundingBox();
  if (menuBox === null) throw new Error('没有量到“更多”。');
  expect(Math.round(menuBox.width)).toBe(390);
  expect(Math.round(menuBox.y + menuBox.height)).toBe(844);
  await expectAccessible(page);
  // Esc 收起来，焦点回到“更多”；点外面也收。
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(more).toBeFocused();
  await more.click();
  await page.mouse.click(195, 200);
  await expect(menu).toBeHidden();

  // 点一项：这一层收起来，对话框打开；关掉以后焦点回到“更多”。
  await more.click();
  await menu.getByRole('button', { name: 'Bring an agent', exact: true }).click();
  await expect(menu).toBeHidden();
  const invite = page.getByRole('dialog', { name: 'Bring an agent' });
  await expect(invite).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(invite).toBeHidden();
  await expect(more).toBeFocused();
  const settings = await openRoomSettings(page);
  await expect(settings).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(settings).toBeHidden();
  await expect(more).toBeFocused();

  // 查找：对话顶上那一行不放它的按钮，从“更多”打开，焦点落在关键词那一格。
  // 开着的时候按钮在那一行上，点了收起来。
  const main = page.locator('.workspace-room-content');
  const searchToggle = main.getByRole('button', { name: 'Search this conversation', exact: true });
  await expect(searchToggle).toBeHidden();
  await more.click();
  await menu.getByRole('button', { name: 'Search this conversation', exact: true }).click();
  await expect(menu).toBeHidden();
  const search = main.getByRole('search', { name: 'Search this conversation', exact: true });
  await expect(search).toBeVisible();
  await expect(search.getByRole('searchbox')).toBeFocused();
  await expect(searchToggle).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  await searchToggle.click();
  await expect(search).toBeHidden();
  await input.click();
  await expect(searchToggle).toBeHidden();

  // 换成场景也在“更多”里。场景的顶栏上照旧是一个按钮，换回来焦点落在“更多”上。
  await more.click();
  await menu.getByRole('button', { name: 'Switch to the scene layout', exact: true }).click();
  await expect(canvas).toBeVisible();
  await expect(input).toBeHidden();
  await page
    .getByRole('button', { name: 'Switch to the conversation layout', exact: true })
    .click();
  await expect(input).toBeInViewport();
  await expect(canvas).toHaveCount(0);
  await expect(more).toBeFocused();
  expect(failures).toEqual([]);
});

test('手机上的下拉从下面出来；没有私聊的房间没有它，一句话都没有时“更多”里没有查找；有人申请时“更多”上带点', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(room);
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await expect(input).toBeInViewport();
  const places = placesButton(page);
  await places.click();
  const placesMenu = page.getByRole('dialog', {
    name: 'Main room and direct messages',
    exact: true,
  });
  const placesBox = await placesMenu.boundingBox();
  if (placesBox === null) throw new Error('没有量到下拉。');
  expect(Math.round(placesBox.width)).toBe(390);
  expect(Math.round(placesBox.y + placesBox.height)).toBe(844);
  await expect(placesMenu.getByRole('button', { name: 'Main room', exact: true })).toHaveAttribute(
    'aria-current',
    'page',
  );
  await expectAccessible(page);
  await placesMenu.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
  await expect(placesMenu).toBeHidden();
  await expect(page.getByRole('heading', { name: 'Build Agent 002', exact: true })).toBeVisible();
  await expect(places).toHaveAccessibleName(/now in Build Agent 002/u);
  await expectNoHorizontalOverflow(page);
  // 这场私聊还一句话没有：“更多”里没有查找。
  const more = page.getByRole('button', { name: 'More', exact: true });
  const moreMenu = page.getByRole('dialog', { name: 'More', exact: true });
  const searchItem = moreMenu.getByRole('button', {
    name: 'Search this conversation',
    exact: true,
  });
  await more.click();
  await expect(moreMenu.getByRole('button', { name: 'Room settings', exact: true })).toBeVisible();
  await expect(searchItem).toHaveCount(0);
  await page.keyboard.press('Escape');
  await openMainRoom(page);
  await expect(page.getByRole('heading', { name: 'Build Agent 002', exact: true })).toBeHidden();
  await more.click();
  await expect(searchItem).toBeVisible();
  await page.keyboard.press('Escape');
  // 看资料时眼前不是对话，也没有查找。
  await openResources(page);
  await more.click();
  await expect(moreMenu.getByRole('button', { name: 'Room settings', exact: true })).toBeVisible();
  await expect(searchItem).toHaveCount(0);
  await page.keyboard.press('Escape');

  // 没有私聊的房间：没有下拉，第一行就是房间名。
  await page.goto('/e2e/fixtures/lobby-scene.html?private&agents=0');
  const top = page.locator('.room-desk__top');
  await expect(
    top.getByRole('heading', { level: 1, name: 'Builders Exchange', exact: true }),
  ).toBeVisible();
  await expect(places).toHaveCount(0);
  await more.click();
  await expect(moreMenu.getByRole('button', { name: 'Bring an agent', exact: true })).toBeVisible();
  await expect(searchItem).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expectNoHorizontalOverflow(page);

  // 有人申请加入：齿轮收在“更多”里，点就标在“更多”上，里面“房间设置”那一项也带着。
  await page.goto('/e2e/fixtures/lobby-scene.html?private&listed&requests');
  await expect(more).toHaveAccessibleDescription('Requests to join: 2');
  await expect(more.locator('.room-desk__attention')).toBeVisible();
  // 这间房不加密，是要一直提醒着的：第二行放不下时房间名让，这一句写全。
  const warning = top.locator('.room-desk__meta > span').first();
  await expect(warning).toHaveText('Public room · Not encrypted');
  expect(await warning.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(
    true,
  );
  await expect(
    top.getByRole('heading', { level: 1, name: 'Builders Exchange', exact: true }),
  ).toBeVisible();
  await more.click();
  const settingsItem = moreMenu.getByRole('button', { name: 'Room settings', exact: true });
  await expect(settingsItem).toHaveAccessibleDescription('Requests to join: 2');
  await expect(settingsItem.locator('.room-desk-menu__attention')).toBeVisible();
  await expect(moreMenu.getByRole('button', { name: 'Share', exact: true })).toBeVisible();
  await expectAccessible(page);
  await page.keyboard.press('Escape');

  // 最窄的手机：顶栏还是一行，不横着滚。
  await page.setViewportSize({ width: 320, height: 640 });
  await page.goto(room);
  await expect(input).toBeInViewport();
  await expect.poll(() => topBarHeight(page)).toBeLessThanOrEqual(60);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

/**
 * 顶栏此刻多高。刚打开的页面还可能整个重画一遍，那一下量不到：量不到算很高，外面接着再量。
 */
async function topBarHeight(page: Page): Promise<number> {
  const box = await page.locator('.room-desk__top').boundingBox();
  return box === null ? Number.POSITIVE_INFINITY : box.height;
}

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

  await useColorScheme(page, 'dark');
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

// 对话顶上“只看”那一行、Agent 之间的长串折起来（1e）。夹具 `&chatter` 是一段有人有 Agent 的对话：
// 你点了第 1 个 Agent，它回你；第 2、1 个 Agent 之间说了 6 句；第 2 个回你一句；Room visitor 点了你；
// 最后第 3、1 个 Agent 各说一句。
const chatter = '/e2e/fixtures/lobby-scene.html?private&chatter&agents=8';
const foldName = '6 more messages between Build Agent 002 and Build Agent 001';

function conversationScope(page: Page, name: 'All' | 'From people' | 'Mentions me') {
  return page
    .locator('.workspace-room-content')
    .getByRole('group', { name: 'Show', exact: true })
    .getByRole('button', { name, exact: true });
}

test('只看：全部、人说的、提到我的；换了就从最新的看起，回到全部折着的还折着', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(chatter);
  const log = page.locator('.workspace-room-content').getByRole('log');
  const messages = log.locator('.conversation-message');
  const fold = log.getByRole('button', { name: foldName, exact: true });
  const all = conversationScope(page, 'All');
  const humans = conversationScope(page, 'From people');
  const addressed = conversationScope(page, 'Mentions me');
  await expect(all).toHaveAttribute('aria-pressed', 'true');
  await expect(humans).toHaveAttribute('aria-pressed', 'false');
  await expect(addressed).toHaveAttribute('aria-pressed', 'false');
  await expect(messages).toHaveCount(6);
  await expect(fold).toBeVisible();
  // 都看过了：“提到我的”上没有条数。
  await expect(addressed.locator('.conversation-panel__scope-count')).toHaveCount(0);

  // 人说的：你自己说的也算，Agent 说的一条不留，也就没有可折的。
  await humans.click();
  await expect(humans).toHaveAttribute('aria-pressed', 'true');
  await expect(all).toHaveAttribute('aria-pressed', 'false');
  await expect(messages).toHaveCount(2);
  await expect(log).toContainText('Take a look at the vendored crate check');
  await expect(log).toContainText('Merged. Have a look at the release notes');
  await expect(fold).toHaveCount(0);
  await expectAccessible(page);

  // 提到我的：点了你名字的，加上回复你那句话的两条；你自己说的不算。
  await addressed.click();
  await expect(addressed).toHaveAttribute('aria-pressed', 'true');
  await expect(messages).toHaveCount(3);
  await expect(log).toContainText('One gap: both image builds skip that directory.');
  await expect(log).toContainText('Fixed: both images copy the directory now');
  await expect(log).toContainText('Merged. Have a look at the release notes');
  await expect(log.locator('.conversation-message--own')).toHaveCount(0);

  await all.click();
  await expect(messages).toHaveCount(6);
  await expect(fold).toHaveAttribute('aria-expanded', 'false');

  // 在“提到我的”里说了话：回到“全部”，刚说的那句看得到。
  await addressed.click();
  await expect(messages).toHaveCount(3);
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Reading them now.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(all).toHaveAttribute('aria-pressed', 'true');
  await expect(log.locator('.conversation-message--own').last()).toContainText('Reading them now.');
  await expect(log.locator('.conversation-message--own').last()).toBeInViewport();
  await useColorScheme(page, 'dark');
  await expectAccessible(page);
  expect(failures).toEqual([]);
});

test('只看：筛出来一条也没有时照实说；往上翻着的时候换一种，落在最新的那条上', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  // 这间房的历史都是 Agent 说的，每隔几句点你一次。
  await page.goto(`${room}&unfolded`);
  const log = page.locator('.workspace-room-content').getByRole('log');
  await expect(log).toContainText('Review milestone 80');
  await conversationScope(page, 'From people').click();
  await expect(
    log.getByText('No person has said anything here yet.', { exact: true }),
  ).toBeVisible();
  await expect(log.locator('.conversation-message')).toHaveCount(0);

  await conversationScope(page, 'All').click();
  await expect(log).toContainText('Review milestone 80');
  await log.evaluate((element) => {
    element.scrollTop = 0;
  });
  await expect(log.getByText('Review milestone 80', { exact: false })).not.toBeInViewport();
  await conversationScope(page, 'Mentions me').click();
  const mentions = log.locator('.conversation-message');
  await expect(mentions.last()).toContainText('Review milestone 79');
  await expect(mentions.last()).toBeInViewport();
  await expect(log.locator('.conversation-message:not([data-mentioned])')).toHaveCount(0);
  await log.evaluate((element) => {
    element.scrollTop = 0;
  });
  await conversationScope(page, 'All').click();
  await expect(log.getByText('Review milestone 80', { exact: false })).toBeInViewport();
  expect(failures).toEqual([]);
});

test('Agent 之间的长串折成一行：点开就在原地，再点收回去；最下面那一串等有人说话才折', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(chatter);
  const log = page.locator('.workspace-room-content').getByRole('log');
  const messages = log.locator('.conversation-message');
  const fold = log.getByRole('button', { name: foldName, exact: true });
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  await expect(messages).toHaveCount(6);
  await expect(log).not.toContainText('Control plane image builds now.');
  // 折着的那一行排在它那几句原来的地方：回你的那一句后面、下一句回你的前面。
  await expect(log.locator('.conversation-message, .conversation-fold').nth(2)).toHaveClass(
    /conversation-fold/u,
  );
  // 最下面那一串（两句）还在进行，不折。
  await expect(log).toContainText('I will draft the release notes.');
  await expect(log).toContainText('Send them to me for a read before posting.');
  await expectAccessible(page);

  await fold.click();
  await expect(fold).toHaveAttribute('aria-expanded', 'true');
  await expect(messages).toHaveCount(12);
  await expect(log).toContainText('Control plane image builds now.');
  await expect(
    log.getByText('Added the license file, rebuilding.', { exact: false }),
  ).toBeVisible();
  await expectAccessible(page);
  // 键盘也行，焦点留在这一行上。（Safari 里鼠标点按钮不给焦点，所以先放上去。）
  await fold.focus();
  await page.keyboard.press('Enter');
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  await expect(fold).toBeFocused();
  await expect(messages).toHaveCount(6);

  // 最下面那一串长到 4 句也不折；有人说了话，它才算说完，折起来。
  for (const text of ['Draft is in the shared notes.', 'Two typos fixed.'])
    await receiveInMainRoom(page, text);
  await expect(messages).toHaveCount(8);
  await expect(log.locator('.conversation-fold')).toHaveCount(1);
  await page.evaluate(() =>
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
      human: true,
      text: 'Thanks, both of you.',
    }),
  );
  await expect(log.locator('.conversation-fold')).toHaveCount(2);
  await expect(
    log.getByRole('button', {
      name: '4 more messages between Build Agent 003 and Build Agent 001',
      exact: true,
    }),
  ).toHaveAttribute('aria-expanded', 'false');
  await expect(messages).toHaveCount(5);
  await expect(log.getByText('Thanks, both of you.', { exact: true })).toBeInViewport();
  expect(failures).toEqual([]);
});

test('折叠能在“对话详情”里整个关掉，这台设备记着；去看折着的某一句时那一串自己打开', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(chatter);
  const main = page.locator('.workspace-room-content');
  const log = main.getByRole('log');
  const messages = log.locator('.conversation-message');
  const fold = log.getByRole('button', { name: foldName, exact: true });
  await expect(fold).toBeVisible();
  const details = main.locator('summary[aria-label="Conversation details"]');
  const folding = main.getByRole('checkbox', { name: /^Fold long runs between agents/u });
  await details.click();
  await expect(folding).toBeChecked();
  await expectAccessible(page);
  await folding.uncheck();
  await expect(fold).toHaveCount(0);
  await expect(messages).toHaveCount(12);

  await page.reload();
  await expect(messages).toHaveCount(12);
  await expect(fold).toHaveCount(0);
  await details.click();
  await expect(folding).not.toBeChecked();
  await folding.check();
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  await expect(messages).toHaveCount(6);

  // 地址里指着折着的那一句（收件箱、查找都是这样跳过来的）：那一串打开，这一句标出来。
  await page.goto(`${chatter}&message=01990d9e-8400-7000-8000-000000002005`);
  await expect(fold).toHaveAttribute('aria-expanded', 'true');
  const focused = log.locator('[data-focused="true"]');
  await expect(focused).toHaveCount(1);
  await expect(focused).toContainText('The gateway image still misses the license file.');
  await expect(focused).toBeInViewport();
  expect(failures).toEqual([]);
});

test('“提到我的”上标着没读的条数：往上翻着时有人点你、回复你才数，看到了就没了', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  // 矮一点的窗口，对话才翻得动。
  await page.setViewportSize({ width: 1440, height: 520 });
  await page.goto(`${chatter}&unfolded`);
  const log = page.locator('.workspace-room-content').getByRole('log');
  const addressed = conversationScope(page, 'Mentions me');
  const count = addressed.locator('.conversation-panel__scope-count');
  await expect(log.getByText('Send them to me for a read before posting.')).toBeInViewport();
  await expect(count).toHaveCount(0);

  await log.evaluate((element) => {
    element.scrollTop = 0;
  });
  await expect(log.getByText('Send them to me for a read before posting.')).not.toBeInViewport();
  // 阅读位置在滚动停下 300ms 后保存；往上翻的那一下也要先落到面板上。
  await page.waitForTimeout(500);
  await page.evaluate(() => {
    const controls = (window as LobbyFixtureWindow).__agentRoomFixtureControls;
    controls.receive({ agentIndex: 2, text: 'Not for you.' });
    controls.receive({ human: true, mentionsYou: true, text: 'Can you approve the notes?' });
    controls.receive({
      agentIndex: 1,
      replyTo: '01990d9e-8400-7000-8000-000000002000',
      text: 'The vendored crate check is in CI now.',
    });
  });
  await expect(count.locator('[aria-hidden="true"]')).toHaveText('2');
  // 名字不变，条数读屏读得到。
  await expect(addressed).toHaveAccessibleDescription('2 unread');
  await expectAccessible(page);

  await addressed.click();
  await expect(log.locator('.conversation-message')).toHaveCount(5);
  await expect(log.getByText('The vendored crate check is in CI now.')).toBeInViewport();
  // 回到全部，落在最新的那条上，算看过了。
  await conversationScope(page, 'All').click();
  await expect(log.getByText('The vendored crate check is in CI now.')).toBeInViewport();
  await expect(count).toHaveCount(0);
  await expect(addressed).toHaveAccessibleDescription('');
  expect(failures).toEqual([]);
});

test('对话顶上那一行：公告在它下面；私聊里没有“只看”，资料照样在；手机上放得下', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${chatter}&notice`);
  const main = page.locator('.workspace-room-content');
  const bar = main.locator('.conversation-panel__bar');
  const scopes = main.getByRole('group', { name: 'Show', exact: true });
  await expect(scopes.getByRole('button')).toHaveCount(3);
  await expect(main.getByRole('button', { name: 'Resources', exact: true })).toBeVisible();
  await expect(
    main.getByRole('button', { name: 'Search this conversation', exact: true }),
  ).toBeVisible();
  // 对话版面里没有“对话 / 资料”两个标签了。
  await expect(page.getByRole('tab', { name: 'Resources', exact: true })).toHaveCount(0);
  const notice = main.locator('.room-notice');
  await expect(notice).toBeVisible();
  const barBox = await bar.boundingBox();
  const noticeBox = await notice.boundingBox();
  const logBox = await main.getByRole('log').boundingBox();
  if (barBox === null || noticeBox === null || logBox === null) throw new Error('没有量到位置。');
  expect(noticeBox.y).toBeGreaterThanOrEqual(barBox.y + barBox.height - 1);
  expect(logBox.y).toBeGreaterThanOrEqual(noticeBox.y + noticeBox.height - 1);
  // 那一行横着铺满主栏，不跟着正文收窄。
  const mainBox = await page.locator('.room-desk__main').boundingBox();
  if (mainBox === null) throw new Error('没有量到主栏。');
  expect(Math.abs(barBox.width - mainBox.width)).toBeLessThanOrEqual(1);
  await expectAccessible(page);

  // 看资料时顶上换成“回到对话”；公告是跟着对话的，这时候不在。
  await openResources(page);
  await expect(notice).toHaveCount(0);
  await expect(scopes).toBeHidden();
  await backToConversationButton(page).click();
  await expect(scopes).toBeVisible();

  // 私聊在主栏里另一块：没有“只看”和折叠的开关，写着这是私聊；资料是这场私聊的资料。
  await page.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
  await expect(page.getByRole('heading', { name: 'Build Agent 002', exact: true })).toBeVisible();
  const desk = page.locator('.room-desk__main');
  await expect(desk.getByRole('group', { name: 'Show', exact: true })).toHaveCount(0);
  await expect(desk.getByText('Only the two of you', { exact: true })).toBeVisible();
  await expect(desk.getByRole('button', { name: 'Resources', exact: true })).toBeVisible();
  // 这场私聊还一句话没有，查找先不出来。
  await expect(
    desk.getByRole('button', { name: 'Search this conversation', exact: true }),
  ).toHaveCount(0);
  await desk
    .locator('summary[aria-label="Conversation details"]')
    .filter({ visible: true })
    .click();
  await expect(desk.getByRole('checkbox', { name: /^Fold long runs/u })).toHaveCount(0);
  await expectAccessible(page);

  // 手机上：三项都在一行里，不横向滚动；折着的那一行放得下。
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(chatter);
  for (const name of ['All', 'From people', 'Mentions me'] as const)
    await expect(conversationScope(page, name)).toBeVisible();
  await expect(main.getByRole('button', { name: 'Resources', exact: true })).toBeVisible();
  const fold = main.getByRole('log').getByRole('button', { name: foldName, exact: true });
  await expect(fold).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  await fold.click();
  await expect(main.getByRole('log').locator('.conversation-message')).toHaveCount(12);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

// “新消息”那条线、按钮上的条数、窗口在不在前台、点回复的那句跳过去（1e 的第二个 PR）。

/** 把窗口当成离开了前台、又回来了：切到别的窗口、别的程序时页面收到的就是这些。 */
async function setWindowFocused(page: Page, focused: boolean): Promise<void> {
  await page.evaluate((focused) => {
    Object.defineProperty(document, 'hasFocus', { configurable: true, value: () => focused });
    window.dispatchEvent(new Event(focused ? 'focus' : 'blur'));
  }, focused);
}

test('往上翻着时来了新消息：按钮上写几条，点了到底，“新消息”线画在这一批上面；你说了话线就撤掉', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  // 矮一点的窗口，对话才翻得动。
  await page.setViewportSize({ width: 1440, height: 520 });
  await page.goto(`${chatter}&unfolded`);
  const main = page.locator('.workspace-room-content');
  const log = main.getByRole('log');
  const line = log.getByRole('separator', { name: 'New', exact: true });
  const latest = main.locator('.conversation-panel__latest');
  await expect(log.getByText('Send them to me for a read before posting.')).toBeInViewport();
  // 打开时都读过了：没有线。
  await expect(line).toHaveCount(0);
  await expect(latest).toHaveCount(0);

  await log.evaluate((element) => {
    element.scrollTop = 0;
  });
  await expect(log.getByText('Send them to me for a read before posting.')).not.toBeInViewport();
  await page.waitForTimeout(500);
  // 两句的时间错开：同一毫秒里来的两句谁在前要看事件编号，编号是随机的。
  await page.evaluate(() => {
    const controls = (window as LobbyFixtureWindow).__agentRoomFixtureControls;
    controls.receive({ agentIndex: 2, ageMs: 20, text: 'Release notes draft is ready.' });
    controls.receive({ human: true, mentionsYou: true, text: 'Can you approve the notes?' });
  });
  await expect(latest).toHaveText('2 new messages');
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  await page.evaluate(() => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
      agentIndex: 1,
      text: 'CI is green on the release branch.',
    });
  });
  await expect(latest).toHaveText('3 new messages');

  await latest.click();
  await expect(log.getByText('CI is green on the release branch.')).toBeInViewport();
  await expect(latest).toHaveCount(0);
  // 只有一条线，下面紧跟着这一批的第一句；看到底了它还留着。
  await expect(line).toHaveCount(1);
  await expect(line.locator('xpath=following-sibling::*[1]')).toContainText(
    'Release notes draft is ready.',
  );
  await expect(line).toBeInViewport();
  await expectAccessible(page);
  await useColorScheme(page, 'dark');
  await expectAccessible(page);

  // 看着的时候来的不挪线。
  await receiveInMainRoom(page, 'Posting them now.');
  await expect(log.getByText('Posting them now.')).toBeInViewport();
  await expect(line.locator('xpath=following-sibling::*[1]')).toContainText(
    'Release notes draft is ready.',
  );

  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Approved.');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(log.locator('.conversation-message--own').last()).toContainText('Approved.');
  await expect(line).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('窗口不在前台时来的话不算看到：窗口标题上写条数，“新消息”线画在它上面；回来就算看到了', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${chatter}&unfolded`);
  const main = page.locator('.workspace-room-content');
  const log = main.getByRole('log');
  const line = log.getByRole('separator', { name: 'New', exact: true });
  await expect(log.getByText('Send them to me for a read before posting.')).toBeInViewport();
  const title = await page.title();
  expect(title).not.toMatch(/^\(/u);

  await setWindowFocused(page, false);
  // 两句的时间错开，谁在前才是定的。
  await page.evaluate(() => {
    const controls = (window as LobbyFixtureWindow).__agentRoomFixtureControls;
    controls.receive({ agentIndex: 2, ageMs: 20, text: 'While you were away.' });
    controls.receive({ agentIndex: 2, text: 'Still away.' });
  });
  await expect(page).toHaveTitle(`(2) ${title}`);
  await expect(line).toHaveCount(1);
  await expect(line.locator('xpath=following-sibling::*[1]')).toContainText('While you were away.');
  // 人还在最底下，只是没看着：新的两句就在眼前，不用按钮。
  await expect(log.getByText('Still away.')).toBeInViewport();
  await expect(main.locator('.conversation-panel__latest')).toHaveCount(0);

  await setWindowFocused(page, true);
  await expect(page).toHaveTitle(title);
  // 回来了线还在，看得出是从哪儿开始没看到的。
  await expect(line).toHaveCount(1);

  // 在看资料时主房间来的话也写在标题上；回到对话就没了。
  await openResources(page);
  await receiveInMainRoom(page, 'While you were in resources.');
  await expect(page).toHaveTitle(`(1) ${title}`);
  await backToConversationButton(page).click();
  await expect(log.getByText('While you were in resources.')).toBeInViewport();
  await expect(page).toHaveTitle(title);
  expect(failures).toEqual([]);
});

test('点回复的是哪句：跳到那一句标出来；它折着的话那一串自己打开；从“提到我的”里点会回到全部', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 520 });
  await page.goto(chatter);
  const main = page.locator('.workspace-room-content');
  const log = main.getByRole('log');
  const fold = log.getByRole('button', { name: foldName, exact: true });
  const focused = log.locator('[data-focused="true"]');
  const fixed = log
    .locator('.conversation-message')
    .filter({ hasText: 'Fixed: both images copy the directory now' });
  await expect(fixed).toBeVisible();
  await expect(focused).toHaveCount(0);

  // “Fixed: …”回复的是你最上面那一句：点引的那一行，翻上去、标出来，焦点也到那儿。
  const quote = fixed.getByRole('button', { name: /Take a look at the vendored crate check/u });
  await expect(quote).toHaveAttribute('title', 'Go to this message');
  await quote.click();
  await expect(focused).toHaveCount(1);
  await expect(focused).toContainText('Take a look at the vendored crate check');
  await expect(focused).toBeInViewport();
  await expect(focused).toBeFocused();
  // 只框住那一句：日期那一行在框外面。
  await expect(focused.locator('.conversation-day')).toHaveCount(0);
  await expect(fold).toHaveAttribute('aria-expanded', 'false');
  await expectAccessible(page);

  // 有人回复了折着的那一串里的一句：点了，那一串自己打开。
  await page.evaluate(() => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
      human: true,
      replyTo: '01990d9e-8400-7000-8000-000000002005',
      text: 'About that license file.',
    });
  });
  const about = log
    .locator('.conversation-message')
    .filter({ hasText: 'About that license file.' });
  await main.locator('.conversation-panel__latest').click();
  await expect(about).toBeInViewport();
  await expect(focused).toHaveCount(0);
  // 用键盘也行。
  await about.getByRole('button', { name: /The gateway image still misses/u }).focus();
  await page.keyboard.press('Enter');
  await expect(fold).toHaveAttribute('aria-expanded', 'true');
  await expect(focused).toContainText('The gateway image still misses the license file.');
  await expect(focused).toBeInViewport();
  await expect(focused).toBeFocused();

  // 在“提到我的”里点：那一句不在筛出来的几条里，回到全部再跳。
  const addressed = conversationScope(page, 'Mentions me');
  await addressed.click();
  await expect(focused).toHaveCount(0);
  await log
    .locator('.conversation-message')
    .filter({ hasText: 'One gap: both image builds skip that directory.' })
    .getByRole('button', { name: /Take a look at the vendored crate check/u })
    .click();
  await expect(conversationScope(page, 'All')).toHaveAttribute('aria-pressed', 'true');
  await expect(focused).toContainText('Take a look at the vendored crate check');
  await expect(focused).toBeInViewport();
  expect(failures).toEqual([]);
});

// 正文里的表格（1f 的第一个 PR）。
test('正文里管道线写的表格：画成表格，宽了在自己的框里左右滚；放得下的不占 Tab，手机上整页不横着滚', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(chatter);
  const log = page.locator('.workspace-room-content').getByRole('log');
  await expect(log.getByText('Send them to me for a read before posting.')).toBeInViewport();
  await page.evaluate(() => {
    const controls = (window as LobbyFixtureWindow).__agentRoomFixtureControls;
    controls.receive({
      agentIndex: 0,
      text: [
        'Image builds:',
        '| Image | State | Time |',
        '| :--- | :---: | ---: |',
        '| Control plane | **passed** | 4m 12s |',
        '| Identity | missing `LICENSE` | 41s |',
        'The gateway one needs a rerun.',
      ].join('\n'),
    });
  });
  const small = log.getByRole('table').last();
  await expect(small).toBeInViewport();
  await expect(small.getByRole('columnheader')).toHaveText(['Image', 'State', 'Time']);
  await expect(small.getByRole('row')).toHaveCount(3);
  await expect(small.getByRole('cell', { name: '4m 12s', exact: true })).toHaveCSS(
    'text-align',
    'right',
  );
  await expect(small.locator('td strong')).toHaveText('passed');
  await expect(small.locator('td code')).toHaveText('LICENSE');
  // 表格上面、下面的字还是各一段。
  const message = log.locator('.conversation-message').filter({ hasText: 'Image builds:' });
  await expect(message.locator('.chat-markdown > p')).toHaveText([
    'Image builds:',
    'The gateway one needs a rerun.',
  ]);
  // 放得下：框不进键盘的顺序，也不当成一个有名字的区域。
  const smallFrame = message.locator('.chat-markdown__table');
  await expect(smallFrame).not.toHaveAttribute('tabindex');
  await expect(
    log.getByRole('group', { name: 'Table, scrolls sideways', exact: true }),
  ).toHaveCount(0);

  await page.evaluate(() => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
      human: true,
      text: [
        '| Check | Runner | Started | Finished | Result | Notes | Follow-up owner | Linked run |',
        '| --- | --- | --- | --- | --- | --- | --- | --- |',
        '| Format, types and tests | ubuntu-latest | 12:01:10 | 12:09:44 | passed | 1592 unit tests | nobody | 43597141197 |',
        '| Web browser acceptance | ubuntu-latest | 12:01:12 | 12:10:23 | flaky | The contrast check ran while the top bar buttons were still halfway through their color transition, so the measured colors were neither the light nor the dark ones. | Build Agent 002 | 43592713434 |',
      ].join('\n'),
    });
  });
  // 比消息那一列宽：框能聚焦、有名字，能用键盘往右滚；字不被拆开，一格里的字多了在格子里折行。
  const wideFrame = log.getByRole('group', { name: 'Table, scrolls sideways', exact: true });
  await expect(wideFrame).toHaveCount(1);
  await expect(wideFrame).toHaveAttribute('tabindex', '0');
  const wide = wideFrame.getByRole('table');
  await expect(wide.getByRole('columnheader')).toHaveCount(8);
  expect(
    await wideFrame.evaluate((element) => element.scrollWidth > element.clientWidth + 100),
  ).toBe(true);
  const stamp = wide.getByRole('cell', { name: '12:01:10', exact: true });
  const notes = wide.getByRole('cell', { name: /^The contrast check ran/u });
  const [stampBox, notesBox] = [await stamp.boundingBox(), await notes.boundingBox()];
  expect(notesBox?.width).toBeLessThanOrEqual(330);
  expect(notesBox?.height).toBeGreaterThan((stampBox?.height ?? 0) * 2);
  await wideFrame.focus();
  await expect(wideFrame).toBeFocused();
  await page.keyboard.press('ArrowRight');
  await expect.poll(() => wideFrame.evaluate((element) => element.scrollLeft)).toBeGreaterThan(0);
  await expectNoHorizontalOverflow(page);
  await expectAccessible(page);
  await useColorScheme(page, 'dark');
  await expectAccessible(page);

  // 手机上：小的那张照样放得下，宽的那张自己滚，整页不横着滚。
  await useColorScheme(page, 'light');
  await page.setViewportSize({ width: 390, height: 760 });
  await expect(small).toBeVisible();
  await expect(smallFrame).not.toHaveAttribute('tabindex');
  await expect(wideFrame).toHaveAttribute('tabindex', '0');
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

type OpenedWindows = Window & { __openedWindows?: unknown[][] };

test('正文里的地址：点了先问，写出网站和完整的地址；取消什么也不开，确认了才开新标签页', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(chatter);
  const log = page.locator('.workspace-room-content').getByRole('log');
  await expect(log.getByText('Send them to me for a read before posting.')).toBeInViewport();
  // 记下页面要开哪些新窗口，不真的开。
  await page.evaluate(() => {
    const opened: unknown[][] = [];
    (window as OpenedWindows).__openedWindows = opened;
    window.open = (...request: unknown[]) => {
      opened.push(request);
      return null;
    };
  });
  const opened = () => page.evaluate(() => (window as OpenedWindows).__openedWindows);
  const run =
    'https://github.com/rainyflash/agent-room/actions/runs/43597141197?check_suite_focus=true';
  await page.evaluate((run) => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
      agentIndex: 0,
      text: [
        `The run: ${run}.`,
        'Docs: [how CI works](https://docs.example.com/ci)',
        'Not these: `https://example.com/in-code`, javascript:alert(1), ftp://example.com/file',
      ].join('\n'),
    });
  }, run);
  const message = log.locator('.conversation-message').filter({ hasText: 'The run:' });
  const links = message.getByRole('link');
  // 能点的只有直接写出来的两个地址；句号、`[字](…)` 的括号不算在地址里。
  await expect(links).toHaveText([run, 'https://docs.example.com/ci']);
  await expect(message.locator('.chat-markdown > p')).toHaveText(
    [
      `The run: ${run}.`,
      'Docs: [how CI works](https://docs.example.com/ci)',
      'Not these: https://example.com/in-code, javascript:alert(1), ftp://example.com/file',
    ].join(''),
  );
  // 不是带地址的链接：中键、右键菜单绕不过确认。
  await expect(message.locator('a')).toHaveCount(0);

  // 键盘走到、回车：先问，网站单独一行，完整的地址原样写着。
  await links.first().focus();
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Open this link?', exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('.external-link__site')).toHaveText('github.com');
  await expect(dialog.locator('.external-link__address')).toHaveText(run);
  await expect(dialog.getByText('It opens in a new tab.', { exact: true })).toBeVisible();
  await expectAccessible(page);
  // Esc 关掉：什么也没开，焦点回到刚才那个地址上。
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(links.first()).toBeFocused();
  expect(await opened()).toEqual([]);

  // 点另一个，取消也什么都不开；确认了才开，开在新标签页里、不带来源。
  await links.nth(1).click();
  await expect(dialog.locator('.external-link__site')).toHaveText('docs.example.com');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toBeHidden();
  expect(await opened()).toEqual([]);
  await links.nth(1).click();
  await dialog.getByRole('button', { name: 'Open', exact: true }).click();
  await expect(dialog).toBeHidden();
  expect(await opened()).toEqual([
    ['https://docs.example.com/ci', '_blank', 'noopener,noreferrer'],
  ]);

  // 暗色下地址和确认框一样看得清。
  await useColorScheme(page, 'dark');
  await expectAccessible(page);
  await links.first().click();
  await expect(dialog).toBeVisible();
  await expectAccessible(page);
  await page.keyboard.press('Escape');

  // 手机上：很长的地址在消息里、确认框里都折行，整页不横着滚。
  await useColorScheme(page, 'light');
  await page.setViewportSize({ width: 390, height: 760 });
  const long = `https://example.com/${'segment-'.repeat(40)}end`;
  await page.evaluate((long) => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
      agentIndex: 2,
      text: long,
    });
  }, long);
  const longLink = log.getByRole('link', { name: long, exact: true });
  await expect(longLink).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await longLink.click({ position: { x: 8, y: 8 } });
  await expect(dialog.locator('.external-link__address')).toHaveText(long);
  await expect(dialog.getByRole('button', { name: 'Open', exact: true })).toBeInViewport();
  await expectNoHorizontalOverflow(page);
  expect(await dialog.evaluate((element) => element.scrollWidth <= element.clientWidth + 1)).toBe(
    true,
  );
  expect(failures).toEqual([]);
});
