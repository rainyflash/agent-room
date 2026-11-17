import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Locator, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { roomAction } from './support/workspace-navigation';

/**
 * 对话里“分享这一段”和房间“分享”里分享过的几段（specs/redesign/project-rooms.md“分享一段对话”，3P-7b）。
 * 夹具 `?private&listed&live=on|off&history`：房间公开、直接进，`history` 是一串“Review milestone N”，最近
 * 一个小时以内说的都在开实况以后。`&historyPage=40` 一开始就有 40 句（41 到 80）。夹具和服务器一样只认开了
 * 实况以后、连着的最多 30 句；`&excerptFail=<错误>` 让分享回那个错误码。
 */
const roomFixture =
  '/e2e/fixtures/lobby-scene.html?private&listed&history&unfolded&view=conversation';
const shareAction = 'Share this stretch';

async function expectAccessible(page: Page, selector: string): Promise<void> {
  const scan = await new AxeBuilder({ page })
    .include(selector)
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({ id, targets: nodes.map(({ target }) => target) })),
  ).toEqual([]);
}

function conversation(page: Page): Locator {
  return page.getByRole('log', { name: /conversation/iu });
}

/** 对话里第 N 句（“Review milestone N: …”）。 */
function milestone(page: Page, index: number): Locator {
  return conversation(page).locator('article', { hasText: `Review milestone ${String(index)}:` });
}

function selectionBar(page: Page): Locator {
  return page.getByRole('region', { name: 'Share a stretch of conversation' });
}

async function beginAt(page: Page, index: number): Promise<void> {
  const message = milestone(page, index);
  await message.scrollIntoViewIfNeeded();
  await message.hover();
  await message.getByRole('button', { name: shareAction }).click();
  await expect(selectionBar(page)).toBeVisible();
}

async function pick(page: Page, index: number): Promise<void> {
  const message = milestone(page, index);
  await message.scrollIntoViewIfNeeded();
  await message.getByRole('button', { name: /^Select this message:/u }).click();
}

for (const width of [1440, 390]) {
  test.describe(`分享这一段：${String(width)}`, () => {
    test.beforeEach(async ({ page }) => {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    });

    test('没放实况时没有这个按钮；放着时每一句都有', async ({ page }) => {
      await page.goto(`${roomFixture}&live=off`);
      await expect(milestone(page, 80)).toBeVisible();
      await milestone(page, 80).hover();
      await expect(conversation(page).getByRole('button', { name: shareAction })).toHaveCount(0);

      await page.goto(`${roomFixture}&live=on`);
      await expect(milestone(page, 80)).toBeVisible();
      await milestone(page, 80).hover();
      await expect(milestone(page, 80).getByRole('button', { name: shareAction })).toBeVisible();
    });

    test('选一段：这一句是开头，点后面的一句是结尾，最多 30 句；Esc 取消', async ({
      page,
    }, testInfo) => {
      const failures = collectPageFailures(page);
      await page.goto(`${roomFixture}&live=on&historyPage=40`);
      await expect(milestone(page, 41)).toBeVisible();

      // 键盘也走得通：开始选以后焦点落在这一句的勾上。
      const start = milestone(page, 41);
      await start.scrollIntoViewIfNeeded();
      await start.getByRole('button', { name: shareAction }).focus();
      await page.keyboard.press('Enter');
      const bar = selectionBar(page);
      await expect(bar).toContainText('1 selected');
      await expect(bar).toContainText('Pick a later message to end the stretch there');
      const startToggle = start.getByRole('button', { name: /^Select this message:/u });
      await expect(startToggle).toBeFocused();
      await expect(startToggle).toHaveAttribute('aria-pressed', 'true');

      // 第 41 到第 80 句是 40 句：够不着，说为什么，选的不变。
      await pick(page, 80);
      await expect(bar).toContainText('1 selected');
      await expect(bar).toContainText('A stretch can be at most 30 messages');

      await pick(page, 70);
      await expect(bar).toContainText('30 selected');
      await expect(conversation(page).locator('article[data-excerpt]')).toHaveCount(30);
      await expectNoHorizontalOverflow(page);
      await expectAccessible(page, '.excerpt-bar');
      await page.screenshot({
        path: testInfo.outputPath(`excerpt-selection-${String(width)}.png`),
      });

      // 点中间的一句：缩到那儿。
      await pick(page, 46);
      await expect(bar).toContainText('6 selected');
      await expect(conversation(page).locator('article[data-excerpt]')).toHaveCount(6);

      await page.keyboard.press('Escape');
      await expect(bar).toHaveCount(0);
      await expect(conversation(page).locator('article[data-excerpt]')).toHaveCount(0);
      await expect(start.getByRole('button', { name: shareAction })).toBeFocused();
      expect(failures).toEqual([]);
    });

    test('分享：对话框里预览这几句、链接和复制、发到别处和二维码；房间“分享”里多了这一段，能删', async ({
      context,
      page,
    }, testInfo) => {
      const failures = collectPageFailures(page);
      await context.grantPermissions(['clipboard-read', 'clipboard-write']);
      await page.goto(`${roomFixture}&live=on`);
      await expect(milestone(page, 75)).toBeVisible();

      await beginAt(page, 75);
      await pick(page, 80);
      await expect(selectionBar(page)).toContainText('6 selected');
      await selectionBar(page).getByRole('button', { name: 'Share', exact: true }).click();

      const dialog = page.getByRole('dialog', { name: 'Share this stretch' });
      const link = dialog.getByRole('textbox', { name: 'Link' });
      await expect(link).toHaveValue(/^https:\/\/agentroom\.test\/r\/aB3xYz\/s\/[A-Za-z0-9]{8}$/u);
      const url = await link.inputValue();
      const preview = dialog.getByRole('list', { name: /^6 messages · /u });
      await expect(preview.getByRole('listitem')).toHaveCount(6);
      await expect(preview.getByRole('listitem').first()).toContainText('Review milestone 75:');
      await expect(preview.getByRole('listitem').last()).toContainText('Review milestone 80:');
      const x = new URL(
        (await dialog
          .getByRole('link', { name: 'Post to X (opens in a new tab)' })
          .getAttribute('href')) ?? '',
      );
      expect(x.searchParams.get('url')).toBe(url);
      await expect(
        dialog.getByRole('img', { name: 'QR code for the link to this stretch' }),
      ).toBeVisible();
      await dialog.getByRole('button', { name: 'Copy link' }).click();
      await expect(dialog.getByRole('button', { name: 'Copied' })).toBeVisible();
      expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(url);
      await expectNoHorizontalOverflow(page);
      await expectAccessible(page, '.room-excerpt-share');
      await page.screenshot({
        path: testInfo.outputPath(`excerpt-share-${String(width)}.png`),
        fullPage: true,
      });

      // 关掉：这一段分享完了，不再选。
      await dialog.getByRole('button', { name: 'Close' }).click();
      await expect(dialog).toBeHidden();
      await expect(selectionBar(page)).toHaveCount(0);

      // 房间“分享”里：你是房主，看得到这间房分享过的全部，新的在最上面、标着是你的。
      await (await roomAction(page, 'Share')).click();
      const roomShare = page.getByRole('dialog', { name: 'Share this room' });
      const shared = roomShare.getByRole('region', { name: 'Shared from this room' });
      await shared.scrollIntoViewIfNeeded();
      await expect(shared).toContainText('you see every stretch shared from this room');
      const rows = shared.getByRole('group');
      await expect(rows).toHaveCount(3);
      await expect(rows.first()).toContainText('6 messages');
      await expect(rows.first()).toContainText('Yours');
      await expect(rows.first().getByRole('link')).toHaveAttribute('href', url);
      await expect(rows.nth(2)).not.toContainText('Yours');
      await expectNoHorizontalOverflow(page);
      await expectAccessible(page, '.room-excerpts');

      // 删掉先在这一行里确认；“不删了”什么都不动。
      await rows.first().getByRole('button', { name: 'Delete' }).click();
      await expect(rows.first()).toContainText('Delete it? The link stops working right away.');
      await expect(rows.first().getByRole('button', { name: 'Keep it' })).toBeFocused();
      await expectAccessible(page, '.room-excerpts');
      await page.screenshot({
        path: testInfo.outputPath(`excerpt-delete-confirm-${String(width)}.png`),
      });
      await rows.first().getByRole('button', { name: 'Keep it' }).click();
      await expect(rows).toHaveCount(3);

      await rows.first().getByRole('button', { name: 'Delete' }).click();
      await rows.first().getByRole('button', { name: 'Delete', exact: true }).click();
      await expect(rows).toHaveCount(2);
      await expect(shared.getByRole('status')).toHaveText('Deleted. The link no longer opens.');
      await expect(shared.getByRole('link', { name: url.replace('https://', '') })).toHaveCount(0);
      expect(failures).toEqual([]);
    });
  });
}

test('分享没成：错误码换成说法，改选的回去还选着；太多了说多久以后再来', async ({ page }) => {
  for (const [code, message] of [
    [
      'before_live',
      'Only messages said after the live view was turned on can be shared. Start from a later message.',
    ],
    ['rate_limited', 'Try again in 20 minutes.'],
    ['not_live', 'This room’s share page isn’t showing a live view right now'],
  ] as const) {
    await page.goto(`${roomFixture}&live=on&excerptFail=${code}`);
    await expect(milestone(page, 79)).toBeVisible();
    await beginAt(page, 79);
    await pick(page, 80);
    await selectionBar(page).getByRole('button', { name: 'Share', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Share this stretch' });
    const alert = dialog.getByRole('alert');
    await expect(alert).toContainText('Couldn’t share this stretch');
    await expect(alert).toContainText(message);
    // 错误码只在“详情”里。
    await expect(alert.getByText(`room_excerpt.`, { exact: false })).toBeHidden();
    await expect(dialog.getByRole('textbox', { name: 'Link' })).toHaveCount(0);
    await expectAccessible(page, '.room-excerpt-share');

    await dialog.getByRole('button', { name: 'Change the selection' }).click();
    await expect(dialog).toBeHidden();
    await expect(selectionBar(page)).toContainText('2 selected');
  }
});
