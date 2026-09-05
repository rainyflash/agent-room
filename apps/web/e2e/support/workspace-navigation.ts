import type { Page } from '@playwright/test';

export async function openRoomSettings(page: Page): Promise<void> {
  const navigation = page.getByRole('button', { name: 'Open conversations', exact: true });
  if (await navigation.isVisible()) await navigation.click();
  await page.locator('.workspace-navigation__settings:visible > summary').click();
}
