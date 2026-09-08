import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
import { expect, test, type Page } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const count of [200, 1000]) {
  test(`${String(count)} 人默认显示附近角色，通过小地图和搜索定位`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width: 1536, height: 1024 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto(`/e2e/fixtures/lobby-scene.html?agents=${String(count)}`);
    await expect(page.locator('canvas')).toBeVisible();
    await expect(page.getByRole('option')).toHaveCount(count);
    await checkMap(page, count);
    await page.screenshot({ path: testInfo.outputPath('nearby.png') });
    await navigateMap(page);
    await expect(page.locator('.room-crowd-group')).toHaveCount(0);
    const host = page.locator('.lobby-scene__pixi');
    await expect
      .poll(async () => Number(await host.getAttribute('data-agent-room-rendered-nodes')))
      .toBeGreaterThan(0);
    expect(Number(await host.getAttribute('data-agent-room-rendered-nodes'))).toBeLessThan(count);
    await page.screenshot({ path: testInfo.outputPath('zoomed.png') });
    await locateLastAgent(page, count);
    await expect(page.locator('.room-minimap')).toBeHidden();
    await page.screenshot({ path: testInfo.outputPath('located.png') });
    await page.getByRole('button', { name: 'Message Agent', exact: true }).click();
    await expect(
      page.locator('.direct-conversation').getByRole('textbox', { name: 'Message', exact: true }),
    ).toBeVisible();
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
  });
}

test('手机 1,000 人可浏览附近、折叠地图及搜索发送消息', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/e2e/fixtures/lobby-scene.html?agents=1000');
  await expect(page.locator('canvas')).toBeVisible();
  await expect
    .poll(async () =>
      Number(
        await page.locator('.lobby-scene__pixi').getAttribute('data-agent-room-rendered-nodes'),
      ),
    )
    .toBeGreaterThan(0);
  await page.screenshot({ path: testInfo.outputPath('mobile-nearby.png') });
  await expect(page.locator('.room-minimap__surface')).toBeHidden();
  await checkMap(page, 1000);
  await page.locator('.room-minimap summary').click();
  await expect(page.locator('.room-minimap__surface')).toBeHidden();
  await page.locator('.room-minimap summary').click();
  await navigateMap(page);
  await expect(page.locator('.room-crowd-group')).toHaveCount(0);
  await locateLastAgent(page, 1000);
  const message = page.getByRole('button', { name: 'Message Agent', exact: true });
  await expect(message).toBeInViewport({ ratio: 1 });
  await page.screenshot({ path: testInfo.outputPath('mobile-located.png') });
  await message.click();
  await expect(
    page.locator('.direct-conversation').getByRole('textbox', { name: 'Message', exact: true }),
  ).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('SVG 降级支持千人地图和定位，默认仅绘制附近人物', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.addInitScript(() => {
    const original: unknown = Reflect.get(HTMLCanvasElement.prototype, 'getContext');
    if (typeof original !== 'function') throw new Error('Canvas context factory missing.');
    Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
      configurable: true,
      value: function (this: HTMLCanvasElement, contextId: string, options?: unknown): unknown {
        return contextId.startsWith('webgl')
          ? null
          : Reflect.apply(original, this, [contextId, options]);
      },
    });
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-scene.html?agents=1000');
  await expect(page.locator('[data-renderer="svg"]')).toBeVisible();
  await checkMap(page, 1000);
  await navigateMap(page);
  await expect(page.locator('.room-crowd-group')).toHaveCount(0);
  const robots = page.locator('[data-renderer="svg"] image[data-character-sprite]');
  await expect.poll(() => robots.count()).toBeGreaterThan(0);
  expect(await robots.count()).toBeLessThan(150);
  await locateLastAgent(page, 1000);
  await page.screenshot({ path: testInfo.outputPath('svg-located.png') });
  await page.setViewportSize({ width: 390, height: 844 });
  const selectedName = page.locator(
    '[data-renderer="svg"] [data-selected="true"] .room-character-name',
  );
  await expect(selectedName).toBeVisible();
  await expect
    .poll(async () => {
      const character = await selectedName.boundingBox();
      const inspector = await page.getByRole('complementary').boundingBox();
      return (
        character !== null &&
        inspector !== null &&
        character.y > 100 &&
        character.y + character.height < inspector.y
      );
    })
    .toBe(true);
  await page.screenshot({ path: testInfo.outputPath('svg-mobile-located.png') });
  expect(failures).toEqual([]);
});

async function locateLastAgent(page: Page, count: number): Promise<void> {
  const name = `Build Agent ${String(count).padStart(3, '0')}`;
  await page.getByRole('button', { name: 'View agents', exact: true }).click();
  // All roster portraits share a compact resource reference, including at 1,000 members.
  const portraits = page.locator('.roster-agent image[data-character-sprite]');
  await expect(portraits).toHaveCount(count);
  const sources = await portraits.evaluateAll((images) =>
    images.map((image) => image.getAttribute('href')),
  );
  expect(new Set(sources).size).toBe(1);
  expect(sources[0]?.length).toBeLessThan(200);
  await page.getByRole('searchbox', { name: 'Search agents', exact: true }).fill(name);
  const member = page.locator('.roster-agent');
  await expect(member).toHaveCount(1);
  await member.click();
  await expect(
    page.getByRole('complementary').getByRole('heading', { name, exact: true }),
  ).toBeVisible();
  await expect(page.locator('.room-crowd-group')).toHaveCount(0);
}

async function checkMap(page: Page, count: number): Promise<void> {
  if (await page.locator('.room-minimap__surface').isHidden())
    await page.locator('.room-minimap summary').click();
  await expect(page.locator('.room-minimap__surface')).toBeVisible();
  await expect(page.locator('[data-map-agent]')).toHaveCount(count);
  await expect(page.locator('.room-crowd-group')).toHaveCount(0);
  // Every dot comes from the same current coordinates as the room, not an invented group.
  const positionsMatch = await page.evaluate(() => {
    const scene = (window as LobbyFixtureWindow).__agentRoomFixtureScene;
    return scene.nodes.every((node) => {
      const dot = document.querySelector(`[data-map-agent="${node.agentId}"]`);
      return (
        dot?.getAttribute('cx') === String(node.x) && dot.getAttribute('cy') === String(node.y)
      );
    });
  });
  expect(positionsMatch).toBe(true);
  await expect(page.getByLabel('Scene zoom', { exact: true })).toHaveText('100%');
}

async function navigateMap(page: Page): Promise<void> {
  const map = page.locator('.room-minimap__surface');
  const viewport = page.locator('.room-minimap__viewport');
  const initialY = await viewport.getAttribute('y');
  const bounds = await map.boundingBox();
  if (bounds === null) throw new Error('Map is not visible.');
  await map.click({ position: { x: bounds.width * 0.75, y: bounds.height * 0.25 } });
  await expect.poll(() => viewport.getAttribute('y')).not.toBe(initialY);
  const afterClickX = await viewport.getAttribute('x');
  await map.press('ArrowLeft');
  await expect.poll(() => viewport.getAttribute('x')).not.toBe(afterClickX);
  await page.getByRole('button', { name: 'Return to your starting view', exact: true }).click();
  await expect.poll(() => viewport.getAttribute('y')).toBe(initialY);
}
