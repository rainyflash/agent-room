import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openRoomSettings } from './support/workspace-navigation';

// 房间页的对话版面（specs/redesign/room-page.md 第 1a 步）。默认还是场景，
// 人在顶栏换过去以后这台设备记着。
const room = '/e2e/fixtures/lobby-scene.html?private&history&agents=8';

function layoutOption(page: Page, name: 'Conversation' | 'Scene') {
  return page
    .getByRole('radiogroup', { name: 'Room layout', exact: true })
    .getByRole('radio', { name, exact: true });
}

async function expectAccessible(page: Page): Promise<void> {
  const accessibility = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(accessibility.violations).toEqual([]);
}

test('默认是场景，换成对话版面以后打开就能看能写，这台设备记着', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(room);
  const canvas = page.locator('.lobby-scene__canvas');
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await expect(canvas).toBeVisible();
  await expect(input).toBeHidden();
  await expect(layoutOption(page, 'Scene')).toHaveAttribute('aria-checked', 'true');

  await layoutOption(page, 'Conversation').click();
  await expect(input).toBeInViewport();
  await expect(canvas).toHaveCount(0);
  await expect(page).toHaveURL(/layout=conversation/u);
  // 顶栏换了一条，焦点接着落在新的切换上。
  await expect(layoutOption(page, 'Conversation')).toBeFocused();
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

  await page.reload();
  await expect(input).toBeInViewport();
  // 地址里没写版面：照这台设备记着的。
  await page.goto(room);
  await expect(input).toBeInViewport();
  await expect(layoutOption(page, 'Conversation')).toHaveAttribute('aria-checked', 'true');

  // 键盘也换得回去。
  await layoutOption(page, 'Conversation').press('ArrowRight');
  await expect(canvas).toBeVisible();
  await expect(input).toBeHidden();
  await expect(page).toHaveURL(/layout=scene/u);
  await expect(layoutOption(page, 'Scene')).toBeFocused();

  // 地址里写了版面就照地址，不管设备记着什么。
  await page.goto(`${room}&layout=conversation`);
  await expect(input).toBeInViewport();
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
  await page.getByRole('button', { name: 'Back to the main room', exact: true }).click();
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
  await expect(canvas).toBeVisible();
  // 手机上两段的切换换成一个按钮。
  await expect(page.getByRole('radiogroup', { name: 'Room layout', exact: true })).toBeHidden();
  await page
    .getByRole('button', { name: 'Switch to the conversation layout', exact: true })
    .click();
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
  // 顶栏的动作一个不少，放不下的那一行自己左右滑。
  await expect(page.getByRole('button', { name: 'Bring an agent', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Room settings', exact: true })).toBeVisible();
  await expectAccessible(page);

  await page.getByRole('button', { name: 'Switch to the scene layout', exact: true }).click();
  await expect(canvas).toBeVisible();
  await expect(input).toBeHidden();
  expect(failures).toEqual([]);
});
