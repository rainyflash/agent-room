import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/room-directory.html';

test('公共房间目录提供从账户工作区进入真实大厅的闭环', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 920, width: 1_440 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { name: 'Find your room' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Default public lobby' })).toBeVisible();
  await expect(page.getByText('2 Agents online')).toBeVisible();
  const enter = page.getByRole('link', { name: 'Enter room' }).first();
  await expect(enter).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await enter.click();
  await expect(page.getByTestId('lobby-route-reached')).toBeVisible();
  expect(failures).toEqual([]);
});

test('窄屏目录不产生横向溢出', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { name: 'Default public lobby' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});
test('搜索支持名称和语言，空结果可清除，导航保留当前页标记', async ({ page }, testInfo) => {
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);
  await expect(page.getByRole('link', { name: 'Rooms', exact: true })).toHaveAttribute(
    'aria-current',
    'page',
  );
  const search = page.getByRole('searchbox', { name: 'Search rooms' });
  await search.fill('  DEFAULT  ');
  await expect(page.getByRole('heading', { name: 'Default public lobby' })).toBeVisible();
  await search.fill('room-that-does-not-exist');
  await expect(page.getByRole('link', { name: 'Enter room' })).toHaveCount(0);
  await expect(page.getByRole('status')).toContainText('No rooms match your search');
  await page.getByRole('button', { name: 'Clear search' }).focus();
  await page.keyboard.press('Enter');
  await expect(search).toHaveValue('');
  await expect(page.getByRole('link', { name: 'Enter room' }).first()).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath('rooms-mobile.png'),
  });
});
