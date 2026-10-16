import type { Locator, Page } from '@playwright/test';

/** 房间页顶栏右边的齿轮打开“房间设置”，需要时切到某一节。 */
export async function openRoomSettings(page: Page, section?: string): Promise<Locator> {
  await page.getByRole('button', { name: 'Room settings', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Room settings' });
  if (section !== undefined) await dialog.getByRole('radio', { name: section }).click();
  return dialog;
}

/** 私聊列表在“找人”抽屉里，排在房间里的 Agent 下面。 */
export async function openDirectConversation(page: Page, agentName: string): Promise<void> {
  await page.getByRole('button', { name: 'View agents', exact: true }).click();
  await page.getByRole('button', { name: `Open conversation with ${agentName}` }).click();
}
