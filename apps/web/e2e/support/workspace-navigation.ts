import type { Page } from '@playwright/test';

/** 窄屏上房间菜单收在按钮后面，先打开它；宽屏上菜单一直在。 */
export async function openRoomMenu(page: Page): Promise<void> {
  const navigation = page.getByRole('button', { name: 'Open room menu', exact: true });
  if (await navigation.isVisible()) await navigation.click();
}

export async function openRoomSettings(page: Page): Promise<void> {
  await openRoomMenu(page);
  await page.locator('.workspace-navigation__settings:visible > summary').click();
}
