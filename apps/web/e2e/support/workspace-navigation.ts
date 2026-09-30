import type { Locator, Page } from '@playwright/test';

/** 窄屏上房间菜单收在按钮后面，先打开它；宽屏上菜单一直在。 */
export async function openRoomMenu(page: Page): Promise<void> {
  const navigation = page.getByRole('button', { name: 'Open room menu', exact: true });
  if (await navigation.isVisible()) await navigation.click();
}

/** 从房间菜单打开“房间设置”，需要时切到某一节。 */
export async function openRoomSettings(page: Page, section?: string): Promise<Locator> {
  await openRoomMenu(page);
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Room settings' });
  if (section !== undefined) await dialog.getByRole('radio', { name: section }).click();
  return dialog;
}
