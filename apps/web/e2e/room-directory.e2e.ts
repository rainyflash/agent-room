import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/room-directory.html';

test('“房间”页只放你的房间，公开房间和公共大厅去发现页找', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 920, width: 1_440 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { level: 1, name: 'Rooms' })).toBeVisible();
  await expect(page.getByTestId('my-rooms-slot')).toBeVisible();
  // 公共大厅不再列在这里。
  await expect(page.getByRole('link', { name: 'Enter room' })).toHaveCount(0);
  const explore = page.getByRole('region', { name: 'Find more rooms' });
  await expect(explore).toContainText('Public rooms people opened');
  await expectNoHorizontalOverflow(page);
  await explore.getByRole('link', { name: 'Go to Explore' }).click();
  await expect(page.getByTestId('explore-route-reached')).toBeVisible();
  expect(failures).toEqual([]);
});

test('窄屏“房间”页不横向溢出，导航标着当前页', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await expect(page.locator('.app-navigation__links [aria-current="page"]')).toHaveAccessibleName(
    'Rooms',
  );
  await expect(page.getByRole('link', { name: 'Go to Explore' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath('rooms-mobile.png'),
  });
  expect(failures).toEqual([]);
});
