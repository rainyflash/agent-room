import { expect, test, type Page } from '@playwright/test';
import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
import { collectPageFailures } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`离开私聊后恢复独立草稿与回复：${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/e2e/fixtures/lobby-scene.html');
    await openExistingConversation(page);
    await page.evaluate(() => {
      (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
        agentIndex: 1,
        roomId: '!direct-002:agent-room.test',
        text: 'A private question from the character.',
      });
    });
    const direct = page.locator('.direct-conversation');
    await direct.getByRole('button', { name: 'Reply to Build Agent 002' }).click();
    const input = direct.getByRole('textbox', { name: 'Message', exact: true });
    await input.fill('Keep this private draft.');
    await page.getByRole('button', { name: 'Room chat', exact: true }).click();
    const publicInput = page
      .locator('.workspace-room-content')
      .getByRole('textbox', { name: 'Message', exact: true });
    await expect(publicInput).toHaveValue('');
    await publicInput.fill('A different public draft.');
    await openExistingConversation(page);
    await expect(input).toHaveValue('Keep this private draft.');
    await expect(direct.getByRole('button', { name: 'Cancel reply' })).toBeVisible();
    await expect(
      direct.getByRole('button', { name: 'Remove mention of Build Agent 002' }),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Return to the room', exact: true }).click();
    await expect(input).toBeHidden();
    await openExistingConversation(page);
    await expect(input).toHaveValue('Keep this private draft.');
    await expect(direct).toHaveCSS('opacity', '1');
    await page.screenshot({
      path: testInfo.outputPath(`restored-private-draft-${String(width)}.png`),
    });
    expect(failures).toEqual([]);
  });
}

test('资料实际显示后标为已读，并且不提前读取隐藏的聊天消息', async ({ page }) => {
  await page.goto('/e2e/fixtures/lobby-scene.html');
  await openExistingConversation(page);
  expect(await displayedEvents(page)).toEqual([]);
  await page.getByRole('tab', { name: 'Resources', exact: true }).click();
  await expect.poll(async () => (await displayedEvents(page)).length).toBe(1);
  await page.getByRole('tab', { name: 'Conversation', exact: true }).click();
  const firstId = await receivePrivate(page, 'First visible private message.');
  await expect
    .poll(async () => await displayedEvents(page))
    .toContainEqual({
      roomId: '!direct-002:agent-room.test',
      matrixEventId: `$fixture-${firstId}`,
    });
  await page.getByRole('tab', { name: 'Resources', exact: true }).click();
  const before = await displayedEvents(page);
  const hiddenId = await receivePrivate(page, 'Message received while reading resources.');
  await expect(
    page.locator('.direct-conversation').getByRole('log', { includeHidden: true }),
  ).toContainText('Message received while reading resources.');
  expect(await displayedEvents(page)).toEqual(before);
  await page.getByRole('tab', { name: 'Conversation', exact: true }).click();
  await expect
    .poll(async () => await displayedEvents(page))
    .toContainEqual({
      roomId: '!direct-002:agent-room.test',
      matrixEventId: `$fixture-${hiddenId}`,
    });
});

test('阅读旧消息时保留未读，显示最新消息后才发送回执', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-scene.html');
  await openExistingConversation(page);
  await page.evaluate(() => {
    const controls = (window as LobbyFixtureWindow).__agentRoomFixtureControls;
    for (let index = 0; index < 35; index += 1)
      controls.receive({
        agentIndex: 1,
        roomId: '!direct-002:agent-room.test',
        text: `Historical private message ${String(index)}.`,
        ageMs: 35_000 - index * 1_000,
      });
  });
  const log = page.locator('.direct-conversation').getByRole('log');
  await expect(log.locator('article')).toHaveCount(35);
  await log.hover();
  await page.mouse.wheel(0, -10_000);
  await expect.poll(async () => await log.evaluate((element) => element.scrollTop)).toBe(0);
  const before = await displayedEvents(page);
  const id = await receivePrivate(page, 'An unread message below the fold.');
  const latest = page.locator('.conversation-panel__latest:visible');
  await expect(latest).toBeVisible();
  expect(await displayedEvents(page)).toEqual(before);
  await latest.click();
  await expect
    .poll(async () => await displayedEvents(page))
    .toContainEqual({
      roomId: '!direct-002:agent-room.test',
      matrixEventId: `$fixture-${id}`,
    });
});

async function openExistingConversation(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Open room menu', exact: true }).click();
  await page.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
  await expect(
    page.locator('.direct-conversation').getByRole('textbox', { name: 'Message', exact: true }),
  ).toBeVisible();
}

async function receivePrivate(page: Page, text: string): Promise<string> {
  return await page.evaluate(
    (text) =>
      (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
        agentIndex: 1,
        roomId: '!direct-002:agent-room.test',
        text,
      }),
    text,
  );
}

async function displayedEvents(page: Page) {
  return await page.evaluate(() =>
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.displayedEvents(),
  );
}
