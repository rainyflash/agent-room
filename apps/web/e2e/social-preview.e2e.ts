import path from 'node:path';

import { expect, test } from '@playwright/test';

import { collectPageFailures } from './support/page-assertions';

/**
 * 分享预览图的底稿（`src/test/social-preview-fixture.tsx`）按 1280×640 画出来、没有报错。
 * 设了 AGENT_ROOM_WRITE_SOCIAL_PREVIEW=1 时把截图写进 `public/social-preview.png`：改了首页说法以后用它重新生成。
 */
test('分享预览图按 1280×640 画出来，标题和首页一样', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 640, width: 1_280 });
  await page.goto('/e2e/fixtures/social-preview.html');

  const card = page.locator('#social-preview');
  await expect(card.getByRole('heading', { level: 1 })).toHaveText(
    'Bring any AI agent into your room with one message.',
  );
  await page.evaluate(async () => {
    await document.fonts.ready;
  });
  expect(await card.boundingBox()).toEqual({ height: 640, width: 1_280, x: 0, y: 0 });

  const output =
    process.env.AGENT_ROOM_WRITE_SOCIAL_PREVIEW === '1'
      ? path.join(testInfo.project.testDir, '..', 'public', 'social-preview.png')
      : testInfo.outputPath('social-preview.png');
  await card.screenshot({ path: output });
  expect(failures).toEqual([]);
});

/**
 * agentroom.chat 首屏的房间插图（`site/room.png`，静态网站用，见 specs/site-discovery/design.md）。
 * 设了 AGENT_ROOM_WRITE_SITE_ART=1 时写进 `site/room.png`：人物或房间的画法变了以后用它重新生成。
 */
test('网站首屏的房间插图按 1152×960 画出来，人物都画上了', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 960, width: 1_152 });
  await page.goto('/e2e/fixtures/social-preview.html?art');

  const art = page.locator('#site-room-art');
  await expect(art.locator('[data-character-sprite="true"]')).toHaveCount(4);
  expect(await art.boundingBox()).toEqual({ height: 960, width: 1_152, x: 0, y: 0 });

  const output =
    process.env.AGENT_ROOM_WRITE_SITE_ART === '1'
      ? path.join(testInfo.project.testDir, '..', 'site', 'room.png')
      : testInfo.outputPath('room.png');
  await art.screenshot({ path: output });
  expect(failures).toEqual([]);
});
