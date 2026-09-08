import { expect, test, type Page } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const count of [200, 1000]) {
  test(`${String(count)} 人可从房间总览放大，再搜索定位到指定成员`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto(`/e2e/fixtures/lobby-scene.html?agents=${String(count)}`);
    await expect(page.locator('canvas')).toBeVisible();
    await expect(page.getByRole('option')).toHaveCount(count);
    await checkOverview(page, count, 48);
    await page.screenshot({ path: testInfo.outputPath('overview.png') });
    await page.locator('.room-crowd-group').first().click();
    await expect(page.locator('.room-crowd-group')).toHaveCount(0);
    const host = page.locator('.lobby-scene__pixi');
    await expect
      .poll(async () => Number(await host.getAttribute('data-agent-room-rendered-nodes')))
      .toBeGreaterThan(0);
    expect(Number(await host.getAttribute('data-agent-room-rendered-nodes'))).toBeLessThan(count);
    await page.screenshot({ path: testInfo.outputPath('zoomed.png') });
    await locateLastAgent(page, count);
    await expect(host).toHaveAttribute('data-agent-room-overview', 'false');
    await page.screenshot({ path: testInfo.outputPath('located.png') });
    await page.getByRole('button', { name: 'Message Agent', exact: true }).click();
    await expect(
      page.locator('.direct-conversation').getByRole('textbox', { name: 'Message', exact: true }),
    ).toBeVisible();
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
  });
}

test('手机 1,000 人总览可点击，搜索后消息操作始终可见', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/e2e/fixtures/lobby-scene.html?agents=1000');
  await checkOverview(page, 1000, 12);
  await page.screenshot({ path: testInfo.outputPath('mobile-overview.png') });
  await page.locator('.room-crowd-group').first().click();
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

test('SVG 降级也支持 1,000 人总览与定位，放大仅绘制视口内人物', async ({ page }, testInfo) => {
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
  await checkOverview(page, 1000, 48);
  await page.locator('.room-crowd-group').first().click();
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

async function checkOverview(page: Page, count: number, maximum: number): Promise<void> {
  const groups = page.locator('.room-crowd-group');
  await expect(groups.first()).toBeVisible();
  expect(await groups.count()).toBeLessThanOrEqual(maximum);
  const points = await groups.evaluateAll((elements) =>
    elements.map((element) => {
      const bounds = element.getBoundingClientRect();
      return { count: Number(element.getAttribute('data-count')), x: bounds.x, y: bounds.y };
    }),
  );
  expect(points.reduce((sum, point) => sum + point.count, 0)).toBe(count);
  for (const [index, point] of points.entries())
    for (const other of points.slice(index + 1))
      expect(Math.hypot(point.x - other.x, point.y - other.y)).toBeGreaterThan(80);
}
