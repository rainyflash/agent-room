import { expect, type Locator, type Page } from '@playwright/test';

/**
 * 房间页顶栏上的一个动作（分享、邀请人、接入 Agent、房间设置、换成场景版面），还有对话顶上的查找。
 * 对话版面在手机上只有一行顶栏，这些都收在“更多”里：先点开“更多”，回的是里面的那一项。
 */
export async function roomAction(page: Page, name: string): Promise<Locator> {
  // 同名的按钮页面里可能还有一个（还没有 Agent 的房间，右栏里也有“接入 Agent”）：顶栏的排在前面。
  const action = page.getByRole('button', { name, exact: true }).filter({ visible: true }).first();
  const more = page.getByRole('button', { name: 'More', exact: true });
  await expect(action.or(more).first()).toBeVisible();
  if (!(await action.isVisible())) await more.click();
  await expect(action).toBeVisible();
  return action;
}

/** 房间页顶栏右边的齿轮（手机上在“更多”里）打开“房间设置”，需要时切到某一节。 */
export async function openRoomSettings(page: Page, section?: string): Promise<Locator> {
  await (await roomAction(page, 'Room settings')).click();
  const dialog = page.getByRole('dialog', { name: 'Room settings' });
  if (section !== undefined) await dialog.getByRole('radio', { name: section }).click();
  return dialog;
}

/**
 * 房间里的 Agent 名单。对话版面够宽时它一直在右栏；窄了，或者在场景版面里，从顶栏的按钮打开抽屉。
 * 回的是此刻看得见的那一份：窄的时候右栏那份还在页面里，只是收着。
 */
export async function openAgentRoster(page: Page): Promise<Locator> {
  const roster = page.locator('.workspace-members').filter({ visible: true });
  const open = page.getByRole('button', { name: 'View agents', exact: true });
  await expect(roster.or(open).first()).toBeVisible();
  if (!(await roster.isVisible())) await open.click();
  await expect(roster).toBeVisible();
  return roster;
}

/** 对话版面里左栏收起来时，房间名旁边的下拉：写着现在在哪儿，点开是主房间和私聊。 */
export function placesButton(page: Page): Locator {
  return page.getByRole('button', { name: /^Go to the main room or a direct message/u });
}

/**
 * 打开和某个 Agent 的私聊。对话版面够宽时私聊列在左栏，窄了在房间名旁边的下拉里；
 * 场景版面里私聊列在 Agent 抽屉里，排在房间里的 Agent 下面。
 */
export async function openDirectConversation(page: Page, agentName: string): Promise<void> {
  const entry = page
    .getByRole('button', { name: `Open conversation with ${agentName}` })
    .filter({ visible: true });
  const places = placesButton(page);
  const agents = page.getByRole('button', { name: 'View agents', exact: true });
  await expect(entry.or(places).or(agents).first()).toBeVisible();
  if (!(await entry.isVisible())) await ((await places.isVisible()) ? places : agents).click();
  await entry.click();
}

/**
 * 对话版面里去看资料：对话顶上那一行右边的“资料”。在私聊里就是那场私聊的资料。
 * 场景版面的面板里还是“对话 / 资料”两个标签，不走这里。
 */
export async function openResources(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Resources', exact: true }).click();
  await expect(backToConversationButton(page)).toBeVisible();
}

/** 对话版面里从资料回到对话。 */
export async function backToConversation(page: Page): Promise<void> {
  await backToConversationButton(page).click();
  await expect(page.getByRole('button', { name: 'Resources', exact: true })).toBeVisible();
}

/** 资料顶上那一行的“回到对话”；有没读的消息时条数读在它的说明里。 */
export function backToConversationButton(page: Page): Locator {
  return page.getByRole('button', { name: 'Back to the conversation', exact: true });
}

/**
 * 对话版面里从私聊回到主房间：左栏在的时候点“主房间”，左栏收起来了点主栏顶上的“回到主房间”
 * （房间名旁边的下拉里也有“主房间”，要先点开，这里不走那条路）。
 */
export async function openMainRoom(page: Page): Promise<void> {
  await page
    .getByRole('button', { name: 'Main room', exact: true })
    .or(page.getByRole('button', { name: 'Back to the main room', exact: true }))
    .filter({ visible: true })
    .click();
}
