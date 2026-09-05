import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`人物发言、未读和对话上下文：${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/e2e/fixtures/lobby-scene.html');
    await expect(
      page.getByRole('button', { name: 'Your character: Fixture operator' }),
    ).toBeVisible();
    await expect(page.locator('canvas')).toBeVisible();
    await selectAgent(page, 'Build Agent 003');
    await page.getByRole('button', { name: 'Close Agent details' }).click();
    await expect(page.getByRole('complementary')).toHaveCount(0);
    const messageId = await page.evaluate(() =>
      (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
        agentIndex: 2,
        text: 'I can help with the room layout.',
      }),
    );
    const bubble = page.getByRole('button', {
      name: 'Build Agent 003: I can help with the room layout.. Open conversation context',
    });
    await expect(bubble).toBeVisible();
    await expect(page.locator('.room-unread')).toHaveText('1');
    await page.screenshot({ path: testInfo.outputPath(`speech-${String(width)}.png`) });
    await bubble.click();
    const context = page.locator(`[data-conversation-message-id="${messageId}"]`);
    await expect(context).toHaveAttribute('data-focused', 'true');
    await expect(context).toBeFocused();
    await expect(context).toBeInViewport();
    await expect(page.locator('.room-unread')).toHaveCount(0);
    expect(
      await page.evaluate(() => (window as LobbyFixtureWindow).__agentRoomFixtureContentReads),
    ).toEqual({ downloads: 0, tickets: 0 });
    const input = page.getByRole('textbox', { name: 'Message', exact: true });
    await input.fill('A draft while looking around.');
    await page.evaluate(() =>
      (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
        agentIndex: 1,
        text: 'Another public message while typing.',
      }),
    );
    await expect(page.getByRole('log')).toContainText('Another public message while typing.');
    await expect(input).toBeFocused();
    await page.getByRole('button', { name: 'Return to the room', exact: true }).click();
    await bubble.click();
    await expect(input).toHaveValue('A draft while looking around.');
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
  });
}

test('自己的公开发言归到人类角色，撤回后气泡消失', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-scene.html');
  await page.getByRole('button', { name: 'Room chat', exact: true }).click();
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await input.fill('Hello everyone in the room.');
  await input.press('Enter');
  await expect(page.getByRole('log')).toContainText('Hello everyone in the room.');
  await page.getByRole('button', { name: 'Return to the room', exact: true }).click();
  const ownBubble = page.locator('.scene-speech[data-speaker="human:@fixture:matrix.test"]');
  await expect(ownBubble).toContainText('Hello everyone in the room.');
  await expect(ownBubble).toBeVisible();
  await expect(page.locator('.room-unread')).toHaveCount(0);
  const id = await page
    .locator('[data-conversation-message-id]')
    .getAttribute('data-conversation-message-id');
  if (id === null) throw new Error('公开发言没有消息标识');
  await page.evaluate((messageId) => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.redact(messageId);
  }, id);
  await expect(ownBubble).toHaveCount(0);
});

test('私聊消息不会出现在房间气泡或未读中，私聊期间仍接收大厅消息', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-scene.html');
  await page.getByRole('button', { name: 'Open room menu', exact: true }).click();
  await page.getByRole('button', { name: 'Open conversation with Build Agent 002' }).click();
  await page.evaluate(() =>
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
      agentIndex: 1,
      roomId: '!direct-002:agent-room.test',
      text: 'This stays private.',
    }),
  );
  await expect(page.locator('.direct-conversation').getByRole('log')).toContainText(
    'This stays private.',
  );
  await expect(page.locator('.scene-speech')).toHaveCount(0);
  await expect(page.locator('.room-unread')).toHaveCount(0);
  await page.evaluate(() =>
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
      agentIndex: 2,
      text: 'A public update arrived.',
    }),
  );
  await expect(page.locator('.room-unread')).toHaveText('1');
  await expect(page.locator('.direct-conversation').getByRole('log')).not.toContainText(
    'A public update arrived.',
  );
  await page.getByRole('button', { name: /^Room chat/u }).click();
  await expect(page.locator('.workspace-room-content').getByRole('log')).toContainText(
    'A public update arrived.',
  );
  await expect(page.locator('.workspace-room-content').getByRole('log')).not.toContainText(
    'This stays private.',
  );
  await expect(page.locator('.room-unread')).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('多人发言有上限、可访问且不互相遮挡', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-scene.html');
  await expect(page.locator('canvas')).toBeVisible();
  await page.evaluate(() => {
    const controls = (window as LobbyFixtureWindow).__agentRoomFixtureControls;
    controls.receive({ agentIndex: 0, text: 'The build is ready.' });
    controls.receive({ agentIndex: 2, text: 'I will review the changes.' });
    controls.receive({ human: true, text: 'Let us discuss them here.' });
  });
  await expect(page.getByRole('button', { name: 'Room participant: Room visitor' })).toBeVisible();
  await expect(page.locator('.scene-speech:visible')).toHaveCount(3);
  const boxes = await page.locator('.scene-speech:visible').evaluateAll((elements) =>
    elements.map((element) => {
      const rect = element.getBoundingClientRect();
      return { x: rect.x, y: rect.y, right: rect.right, bottom: rect.bottom };
    }),
  );
  for (const [index, box] of boxes.entries()) {
    for (const other of boxes.slice(index + 1))
      expect(
        box.right <= other.x ||
          other.right <= box.x ||
          box.bottom <= other.y ||
          other.bottom <= box.y,
      ).toBe(true);
  }
  const accessibility = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(accessibility.violations).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath('multiple-speakers.png') });
});

test('SVG 降级保留人类、气泡和成员增量位置', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.addInitScript(() => {
    Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
      configurable: true,
      value: () => null,
    });
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/e2e/fixtures/lobby-scene.html');
  await expect(page.locator('[data-renderer="svg"]')).toBeVisible();
  const character = page.locator('[data-character-id="01990d9e-8400-7000-8000-000000000003"]');
  const position = await character.getAttribute('transform');
  if (position === null) throw new Error('场景人物缺少空间坐标');
  const newAgent = await page.evaluate(() =>
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.joinAgent(),
  );
  await expect(page.getByRole('option')).toHaveCount(25);
  await expect(character).toHaveAttribute('transform', position);
  await page.evaluate((id) => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.leaveAgent(id);
  }, newAgent);
  await expect(page.getByRole('option')).toHaveCount(24);
  await expect(character).toHaveAttribute('transform', position);
  await expect(page.locator('[data-character-id="human:@fixture:matrix.test"]')).toBeVisible();
  await page.evaluate(() =>
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.receive({
      agentIndex: 2,
      text: 'SVG conversation works too.',
    }),
  );
  await page.getByRole('button', { name: /Build Agent 003: SVG conversation works too/u }).click();
  await expect(page.locator('[data-focused="true"]')).toContainText('SVG conversation works too.');
  expect(failures).toEqual([]);
});

async function selectAgent(page: Page, name: string): Promise<void> {
  await page.getByRole('button', { name: 'Find a character', exact: true }).click();
  const members = page.getByRole('dialog', { name: 'Agents in this room', exact: true });
  await members.getByRole('searchbox', { name: 'Search agents' }).fill(name);
  await members.getByRole('button', { name: new RegExp(name, 'u') }).click();
  await expect(page.getByRole('complementary')).toContainText(name);
}
