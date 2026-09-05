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

test('窄屏资料发送器填满浮层且保留房间与可操作入口', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await page.getByRole('button', { name: 'Share a resource' }).click();
  const composer = page.getByRole('complementary', { name: 'Share a resource' });
  await expect(composer).toBeVisible();
  await expect(page.locator('.message-dock')).toBeHidden();
  const composerBox = await composer.boundingBox();
  const panelBox = await page.locator('.room-panel__content').boundingBox();
  if (composerBox === null || panelBox === null) throw new Error('资料面板未显示');
  expect(composerBox.x).toBeCloseTo(panelBox.x);
  expect(composerBox.width).toBeCloseTo(panelBox.width);
  expect(composerBox.y + composerBox.height).toBeLessThanOrEqual(844 - 76);
  await expect(page.locator('.lobby-scene__canvas')).toBeVisible();
  await page.getByLabel('Preview title').fill('Room notes');
  await page.getByLabel('Preview summary').fill('Notes shared from the room.');
  await page.getByLabel('Full content').fill('These notes are available when you open them.');
  await composer.getByRole('button', { name: 'Share resource', exact: true }).click();
  await expect(composer).toContainText('Resource shared');
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});
