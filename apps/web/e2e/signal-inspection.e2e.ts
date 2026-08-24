import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html';

test('信号坞默认单行，正文必须经显式点击才读取并保持惰性', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  const dock = page.getByRole('region', { name: 'Signal dock' });
  await expect(dock).toBeVisible();
  const compactBox = await dock.boundingBox();
  expect(compactBox?.height).toBeLessThanOrEqual(60);
  expect(await fixtureContentReads(page)).toEqual({ downloads: 0, tickets: 0 });

  await page.getByRole('button', { name: 'Expand signal timeline' }).click();
  const timeline = page.getByRole('list', { name: 'Room signal timeline' });
  await expect(timeline.getByRole('button')).toHaveCount(2);
  await timeline.getByRole('button', { name: /Protocol review ready/u }).click();

  const inspector = page.getByRole('complementary', { name: 'Protocol review ready' });
  await expect(inspector).toContainText('Only preview metadata is loaded');
  expect(await fixtureContentReads(page)).toEqual({ downloads: 0, tickets: 0 });

  await inspector.getByRole('button', { name: 'Open full content' }).click();
  await expect(inspector).toContainText('Length and SHA-256 verified');
  expect(await fixtureContentReads(page)).toEqual({ downloads: 1, tickets: 1 });
  await expect(inspector.getByText('<img src=x onerror=alert(1)>')).toBeVisible();
  await expect(inspector.getByText('[link](javascript:alert(1))')).toBeVisible();
  await expect(inspector.locator('.restricted-markdown img')).toHaveCount(0);
  await expect(inspector.locator('.restricted-markdown a')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);

  await page.screenshot({
    animations: 'disabled',
    path: '../../artifacts/browser/task-21/playwright-content-inspector.png',
  });
});

type FixtureContentReads = { readonly downloads: number; readonly tickets: number };

async function fixtureContentReads(page: Page): Promise<FixtureContentReads> {
  return await page.evaluate(() => {
    const value: unknown = Reflect.get(window, '__agentRoomFixtureContentReads');
    if (typeof value !== 'object' || value === null) {
      throw new Error('正文读取夹具计数器缺失。');
    }
    const record = value as Record<string, unknown>;
    if (typeof record.downloads !== 'number' || typeof record.tickets !== 'number') {
      throw new Error('正文读取夹具计数器无效。');
    }
    return { downloads: record.downloads, tickets: record.tickets };
  });
}
