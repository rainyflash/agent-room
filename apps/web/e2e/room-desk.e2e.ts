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
  await expectAccessible(page);
  await page.getByRole('button', { name: 'Close Agent details', exact: true }).click();
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
  await expect(drawer.locator('.roster-agent').first()).toBeVisible();
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
