import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html?view=space';
const wcagTags = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] as const;

test('大厅通过 WCAG 2.2 AA 自动扫描与纯键盘主流程', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  const scene = page.getByRole('listbox', { name: 'Interactive Agent room scene' });
  await scene.focus();
  await page.keyboard.press('ArrowRight');
  const activeOptionId = await scene.getAttribute('aria-activedescendant');
  expect(activeOptionId).not.toBeNull();
  await expect(page.locator(`#${activeOptionId ?? 'missing-option'}`)).toHaveAttribute(
    'aria-selected',
    'true',
  );

  await page.keyboard.press('Enter');
  const closeButton = page.getByRole('button', { name: 'Close Agent details' });
  await expect(closeButton).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(scene).toBeFocused();
  await expect(page.getByRole('complementary')).toHaveCount(0);

  const results = await new AxeBuilder({ page }).withTags([...wcagTags]).analyze();
  expect(results.violations).toEqual([]);
  expect(failures).toEqual([]);
});

test('高对比模式强制进入完整列表且保持可操作', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.emulateMedia({ forcedColors: 'active', reducedMotion: 'reduce' });
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  await expect(page.locator('canvas')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Agent roster' })).toBeVisible();
  const firstAgent = page.getByRole('button', { name: /Build Agent 001/u });
  await firstAgent.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Close Agent details' })).toBeFocused();

  const results = await new AxeBuilder({ page }).withTags([...wcagTags]).analyze();
  expect(results.violations).toEqual([]);
  expect(failures).toEqual([]);
});

test('等效 200% 放大视口下无 Canvas 也能完成核心任务', async ({ page }) => {
  const failures = collectPageFailures(page);
  // 1280 CSS 像素宽窗口在 200% 放大后的可用布局宽度等效为 640 CSS 像素。
  await page.setViewportSize({ height: 720, width: 640 });
  await page.goto(fixturePath);
  await page.getByRole('button', { name: 'List view', exact: true }).click();

  await expect(page.locator('canvas')).toHaveCount(0);
  const firstAgent = page.getByRole('button', { name: /Build Agent 001/u });
  await firstAgent.focus();
  await page.keyboard.press('ArrowDown');
  await expect(page.locator('.roster-agent').filter({ hasText: 'Build Agent 002' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary')).toContainText('Build Agent 002');
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});
