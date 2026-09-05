import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import type { LobbySceneProjection } from '../src/features/lobby/domain/scene-projection';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`游戏房间、按需对话、草稿和成员抽屉：${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto('/e2e/fixtures/lobby-scene.html');
    await expect(
      page.getByRole('heading', { name: 'Builders Exchange', exact: true }),
    ).toBeVisible();
    const canvas = page.locator('.lobby-scene__canvas');
    await expect(canvas).toBeVisible();
    const originalCanvas = await canvas.elementHandle();
    const input = page.getByRole('textbox', { name: 'Message', exact: true });
    await expect(input).toBeHidden();
    await expectAccessibleWorkspace(page);
    await page.screenshot({ path: testInfo.outputPath(`room-${String(width)}.png`) });
    await page.getByRole('button', { name: 'Room chat', exact: true }).click();
    await expect(input).toBeInViewport();
    await input.fill('Hello from the room.');
    await page.getByRole('button', { name: 'Return to the room', exact: true }).click();
    await expect(input).toBeHidden();
    await expect(page.getByRole('listbox')).toBeFocused();
    await page.getByRole('button', { name: 'Room chat', exact: true }).click();
    await expect(input).toHaveValue('Hello from the room.');
    await page.getByRole('tab', { name: 'Resources', exact: true }).click();
    await expect(
      page.getByRole('heading', { name: 'Shared with the room', exact: true }),
    ).toBeVisible();
    await page.getByRole('tab', { name: 'Resources', exact: true }).press('ArrowLeft');
    await expect(page.getByRole('tab', { name: 'Conversation', exact: true })).toBeFocused();
    await expect(input).toHaveValue('Hello from the room.');
    expect(
      await originalCanvas.evaluate(
        (element) => element === document.querySelector('.lobby-scene__canvas'),
      ),
    ).toBe(true);
    await expectAccessibleWorkspace(page);
    await page.getByRole('button', { name: 'Return to the room', exact: true }).click();
    const findCharacter = page.getByRole('button', { name: 'Find a character', exact: true });
    await findCharacter.click();
    const members = page.getByRole('dialog', { name: 'Agents in this room', exact: true });
    await members.getByRole('searchbox', { name: 'Search agents' }).fill('Build Agent 003');
    await expect(members.locator('.roster-agent')).toHaveCount(1);
    await members.getByRole('button', { name: 'Close panel', exact: true }).press('Escape');
    await expect(members).not.toBeVisible();
    await expect(findCharacter).toBeFocused();
    await findCharacter.click();
    await members.getByRole('searchbox', { name: 'Search agents' }).fill('Build Agent 003');
    await members.getByRole('button', { name: /Build Agent 003/u }).click();
    await expect(page.getByRole('complementary')).toContainText('Build Agent 003');
    await expect(page.getByRole('complementary')).toHaveCSS('opacity', '1');
    await expectAccessibleWorkspace(page);
    await page.getByRole('button', { name: 'Close Agent details', exact: true }).click();
    await expect(page.getByRole('complementary')).toHaveCount(0);
    await expect(page.getByRole('listbox')).toBeFocused();
    await expectNoHorizontalOverflow(page);
    await page.getByRole('button', { name: 'Shared resources', exact: true }).click();
    await page.reload();
    await expect(page.getByRole('tab', { name: 'Resources', exact: true })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await expect(canvas).toBeVisible();
    await expectAccessibleWorkspace(page);
    expect(failures).toEqual([]);
  });
}

test('点击真实人物打开私聊，场景继续活动并支持缩放拖动', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/e2e/fixtures/lobby-scene.html');
  const canvas = page.locator('.lobby-scene__canvas');
  const host = page.locator('.lobby-scene__pixi');
  await expect(canvas).toBeVisible();
  const target = await page.evaluate(() => {
    const fixture = window as Window & { readonly __agentRoomFixtureScene?: LobbySceneProjection };
    const node = fixture.__agentRoomFixtureScene?.nodes.find(
      (entry) => entry.displayName === 'Build Agent 003',
    );
    const bounds = document.querySelector('.lobby-scene__canvas')?.getBoundingClientRect();
    if (node === undefined || bounds === undefined) throw new Error('测试房间没有目标角色');
    const scale = Math.max(0.22, Math.min((bounds.width - 44) / 2600, (bounds.height - 44) / 1500));
    return {
      x: bounds.x + (bounds.width - 2600 * scale) / 2 + node.x * scale,
      y:
        bounds.y +
        (bounds.height - 1500 * scale) / 2 +
        (node.y - 42 * Math.max(0.83, node.radius / 27)) * scale,
    };
  });
  await page.mouse.click(target.x, target.y);
  await expect(page.getByRole('complementary')).toContainText('Build Agent 003');
  await page.getByRole('button', { name: 'Message Agent', exact: true }).click();
  const direct = page.locator('.direct-conversation');
  await expect(direct.getByRole('heading', { name: 'Build Agent 003' })).toBeVisible();
  await direct
    .getByRole('textbox', { name: 'Message', exact: true })
    .fill('Hello, I found you in the room.');
  await direct.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(direct.getByRole('log')).toContainText('Hello, I found you in the room.');
  await expect(canvas).toBeVisible();
  await page.getByRole('button', { name: 'Return to the room', exact: true }).click();
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await expect(host).toHaveAttribute('data-agent-room-motion', 'active');
  const frame = Number((await host.getAttribute('data-agent-room-animation-frame')) ?? 0);
  await expect
    .poll(async () => Number(await host.getAttribute('data-agent-room-animation-frame')))
    .toBeGreaterThan(frame + 2);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(host).toHaveAttribute('data-agent-room-motion', 'paused');
  const frozen = await host.getAttribute('data-agent-room-animation-frame');
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  const zoom = await page.locator('.signal-dock__zoom output').innerText();
  await canvas.hover({ position: { x: 400, y: 450 } });
  await page.mouse.wheel(0, -200);
  await expect(page.locator('.signal-dock__zoom output')).not.toHaveText(zoom);
  await page.mouse.move(400, 450);
  await page.mouse.down();
  await page.mouse.move(580, 490, { steps: 10 });
  await page.mouse.up();
  await expect(page.getByRole('complementary')).toHaveCount(0);
  expect(await host.getAttribute('data-agent-room-animation-frame')).toBe(frozen);
  expect(failures).toEqual([]);
});

async function expectAccessibleWorkspace(page: Page): Promise<void> {
  const accessibility = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(accessibility.violations).toEqual([]);
}
