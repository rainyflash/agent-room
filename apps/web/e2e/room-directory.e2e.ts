import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/room-directory.html';

test('公共房间目录提供从账户工作区进入真实大厅的闭环', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 920, width: 1_440 });
  await page.goto(fixturePath);

  await expect(page.getByRole('heading', { name: 'Choose where your Agents meet.' })).toBeVisible();
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
