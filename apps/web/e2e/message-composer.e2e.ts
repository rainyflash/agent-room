import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html?view=resources';

test('资料视图内可通过用户 Matrix 会话分享带摘要的资料', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  const launcher = page.getByRole('button', { name: 'Share a resource' });
  await expect(launcher).toBeInViewport();
  await expect(page.getByRole('tab', { name: 'Resources', exact: true })).toHaveAttribute(
    'aria-selected',
    'true',
  );

  await launcher.click();
  const composer = page.getByRole('complementary', { name: 'Share a resource' });
  await expect(composer).toBeVisible();
  await expect(composer).toContainText('Fixture operator');
  await expect(composer).toContainText('Builders Exchange');

  await page.getByLabel('Preview title').fill('Protocol review');
  await page.getByLabel('Preview summary').fill('Please inspect the protocol change.');
  await page.getByLabel('Full content').fill('Review https://example.com and keep <script> inert.');
  await expect(composer).toContainText('External links detected');
  await expect(composer).toContainText('HTML markup will remain inert');

  await page.getByRole('button', { name: 'Share resource' }).click();
  await expect(composer).toContainText('Resource shared');
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('窄屏发送器占满可用宽度且保留底部安全区', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await page.getByRole('button', { name: 'Share a resource' }).click();
  const composer = page.getByRole('complementary', { name: 'Share a resource' });
  await expect(composer).toBeVisible();
  await expect(page.locator('.message-dock')).toBeHidden();
  const composerBox = await composer.boundingBox();
  expect(composerBox).not.toBeNull();
  expect(composerBox?.x).toBe(0);
  expect(composerBox?.width).toBe(390);
  expect((composerBox?.y ?? 0) + (composerBox?.height ?? 0)).toBeLessThanOrEqual(844);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});
