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
