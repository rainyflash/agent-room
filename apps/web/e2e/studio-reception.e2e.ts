import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const studio = '/e2e/fixtures/lobby-scene.html?agents=6';

test('工作室只投影在场人物，离线成员仍可查看并留言', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1536, height: 1024 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto(studio);
  await expect(page.locator('canvas')).toBeVisible();
  await expect(page.getByRole('option')).toHaveCount(4);
  await expect(page.locator('.workspace-header')).toContainText('4 agents present');
  await page.screenshot({ path: testInfo.outputPath('studio-desktop.png') });

  await selectMember(page, 'Echo');
  const inspector = page.getByRole('complementary');
  await expect(inspector.getByRole('heading', { name: 'Echo' })).toBeVisible();
  await expect(inspector.locator('.agent-reception')).toHaveAttribute('data-state', 'away');
  await expect(page.getByRole('option', { name: /Echo/u })).toHaveCount(0);
  await page.screenshot({ path: testInfo.outputPath('studio-away.png') });
  await inspector.getByRole('button', { name: 'Leave a message', exact: true }).click();
  const conversation = page.locator('.direct-conversation');
  await expect(conversation.getByRole('heading', { name: 'Echo' })).toBeVisible();
  await conversation
    .getByRole('textbox', { name: 'Message', exact: true })
    .fill('Please reply when you return.');
  await conversation.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(conversation.getByRole('log')).toContainText('Please reply when you return.');
  await expect(conversation.getByRole('status')).toContainText(
    'This does not confirm that the recipient has read it.',
  );
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('选中的人物离场后详情保留，连接与接待分别更新', async ({ page }) => {
  await page.goto(studio);
  await selectMember(page, 'Mira');
  const inspector = page.getByRole('complementary');
  await expect(inspector.locator('.agent-reception')).toHaveAttribute('data-state', 'unknown');
  await page.evaluate(() => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.setAgentStatus(
      '01990d9e-8400-7000-8000-000000000001',
      'offline',
    );
  });
  await expect(page.getByRole('option')).toHaveCount(3);
  await expect(inspector.getByRole('heading', { name: 'Mira' })).toBeVisible();
  await expect(inspector.locator('.agent-reception')).toHaveAttribute('data-state', 'away');
  await page.getByRole('button', { name: 'Close Agent details' }).click();

  await selectMember(page, 'Atlas');
  await expect(inspector.locator('.agent-reception')).toHaveAttribute('data-state', 'recent');
  await page.evaluate(() => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.advancePresenceClock(36_000);
  });
  await expect(inspector.locator('.agent-reception')).toHaveAttribute('data-state', 'waiting');
  await expect(page.locator('.lobby-scene__pixi')).toHaveAttribute(
    'data-agent-room-motion',
    'paused',
  );
  await expect(page.getByRole('option', { name: /Atlas/u })).toHaveCount(1);
  await page.evaluate(() => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.advancePresenceClock(270_000);
  });
  await expect(inspector.locator('.agent-reception')).toHaveAttribute('data-state', 'reconnecting');
  await expect(page.getByRole('option', { name: /Atlas/u })).toHaveCount(1);
  await page.evaluate(() => {
    (window as LobbyFixtureWindow).__agentRoomFixtureControls.advancePresenceClock(31_000);
  });
  await expect(inspector.locator('.agent-reception')).toHaveAttribute('data-state', 'away');
  await expect(page.getByRole('option')).toHaveCount(0);
  await expect(page.getByText('No agents are here right now')).toBeVisible();
});

test('手机工作室和人物详情可操作，控件不溢出或遮挡', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto(studio);
  await expect(page.locator('canvas')).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath('studio-mobile.png') });
  await selectMember(page, 'Atlas');
  await expect(page.getByRole('complementary')).toContainText('Recently checked messages');
  const message = page
    .getByRole('complementary')
    .getByRole('button', { name: 'Message Agent', exact: true });
  await expect(message).toBeInViewport({ ratio: 1 });
  await page.screenshot({ path: testInfo.outputPath('studio-mobile-inspector.png') });
  await expectNoHorizontalOverflow(page);
  const accessibility = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .analyze();
  expect(accessibility.violations).toEqual([]);
  await message.click();
  await expect(
    page.locator('.direct-conversation').getByRole('textbox', { name: 'Message', exact: true }),
  ).toBeVisible();
  expect(failures).toEqual([]);
});

test('无 WebGL 时沿用同一套房间和人物素材', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.addInitScript(() => {
    const getContext: unknown = Reflect.get(HTMLCanvasElement.prototype, 'getContext');
    if (typeof getContext !== 'function') throw new Error('Canvas context factory missing.');
    Object.defineProperty(HTMLCanvasElement.prototype, 'getContext', {
      configurable: true,
      value: function (this: HTMLCanvasElement, contextId: string, options?: unknown) {
        if (contextId.startsWith('webgl')) return null;
        const context: unknown = Reflect.apply(getContext, this, [contextId, options]);
        if (
          context === null ||
          context instanceof CanvasRenderingContext2D ||
          context instanceof ImageBitmapRenderingContext
        )
          return context;
        throw new Error('Unexpected canvas context in the SVG fixture.');
      },
    });
  });
  await page.setViewportSize({ width: 1536, height: 1024 });
  await page.goto(studio);
  await expect(page.locator('[data-renderer="svg"]')).toBeVisible();
  await expect(page.locator('[data-renderer="svg"] image[data-character-sprite]')).toHaveCount(4);
  await expect(page.locator('[data-renderer="svg"] polygon[fill="#173544"]')).toHaveCount(1);
  await page.screenshot({ path: testInfo.outputPath('studio-svg.png') });
  const scene = page.getByRole('listbox');
  await scene.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('complementary')).toBeVisible();
  expect(failures).toEqual([]);
});

async function selectMember(page: Page, name: string): Promise<void> {
  await page.getByRole('button', { name: 'View agents', exact: true }).click();
  await page.locator('.roster-agent').filter({ hasText: name }).click();
}
