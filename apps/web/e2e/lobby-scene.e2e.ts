import { expect, test, type Page } from '@playwright/test';

const fixturePath = '/e2e/fixtures/lobby-scene.html';

test('200 个 Agent 的全幅场景、键盘导航与焦点恢复可用', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  const scene = page.getByRole('application', { name: 'Interactive Agent room scene' });
  await expect(scene.locator('canvas')).toBeVisible();
  await expect(page.locator('.room-beacon')).toContainText('200 agents');
  await expect(page.locator('.sr-only li')).toHaveCount(200);

  await scene.focus();
  await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('complementary')).toBeVisible();
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-20/playwright-lobby-inspector.png',
  });
  await page.getByRole('button', { name: 'Close Agent details' }).click();
  await expect(scene).toBeFocused();
  await expect(page.getByRole('complementary')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-20/playwright-lobby-desktop.png',
  });
});

test('200 节点场景交互保持在有界帧预算内', async ({ page }, testInfo) => {
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);
  const canvas = page.locator('.lobby-scene__canvas');
  await expect(canvas).toBeVisible();

  const frameDurations = await canvas.evaluate(async (element) => {
    const samples: number[] = [];
    for (let index = 0; index < 72; index += 1) {
      const startedAt = performance.now();
      element.dispatchEvent(
        new WheelEvent('wheel', {
          bubbles: true,
          cancelable: true,
          clientX: element.clientWidth / 2,
          clientY: element.clientHeight / 2,
          deltaY: index % 12 < 6 ? -5 : 5,
        }),
      );
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          resolve();
        });
      });
      samples.push(performance.now() - startedAt);
    }
    return samples.slice(6).toSorted((left, right) => left - right);
  });

  const median = percentile(frameDurations, 0.5);
  const p95 = percentile(frameDurations, 0.95);
  await testInfo.attach('frame-budget.json', {
    body: Buffer.from(JSON.stringify({ medianMilliseconds: median, p95Milliseconds: p95 })),
    contentType: 'application/json',
  });
  testInfo.annotations.push({
    description: `median=${median.toFixed(2)}ms, p95=${p95.toFixed(2)}ms`,
    type: 'performance',
  });
  expect(median).toBeLessThanOrEqual(22);
  expect(p95).toBeLessThanOrEqual(40);
});

test('手机默认使用完整列表且 reduced-motion 不保留持续动画', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { name: 'Agent roster' })).toBeVisible();
  await expect(page.locator('.signal-dock')).toHaveCount(0);
  await expect(page.locator('.list-roster__list > li')).toHaveCount(200);
  await expect(page.locator('canvas')).toHaveCount(0);
  await expect(page.locator('.ar-status-mark--pulse')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-20/playwright-lobby-mobile.png',
  });
});

function percentile(values: readonly number[], ratio: number): number {
  const index = Math.min(values.length - 1, Math.floor(values.length * ratio));
  return values[index] ?? Number.POSITIVE_INFINITY;
}

function collectPageFailures(page: Page): string[] {
  const failures: string[] = [];
  page.on('pageerror', (error) => {
    failures.push(error.message);
  });
  page.on('console', (message) => {
    if (message.type() === 'error') {
      failures.push(message.text());
    }
  });
  return failures;
}

async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  const dimensions = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
}
