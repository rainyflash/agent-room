import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const roomFixture = '/e2e/fixtures/lobby-scene.html';

for (const width of [1440, 390]) {
  test(`置顶：须知下面一行，点开列出每一句，点一句跳到对话里那一句（没加载的往前补）：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private&pins&history&view=conversation`);

    const pins = page.getByRole('region', { name: 'Pinned messages' });
    const log = page.getByRole('log', { name: /conversation/iu });
    const toggle = pins.getByRole('button', { name: /Show pinned messages/u });
    // 平时一行：几条、最新那一条是谁说的、开头；改了你还没看过，亮一个点。
    await expect(toggle).toContainText('Pinned');
    await expect(pins.locator('.room-pins__count')).toHaveText('2');
    await expect(pins.locator('.room-pins__summary')).toHaveText(
      /^.+: Review milestone 79: implementation and verification details\.$/u,
    );
    await expect(pins.getByText('Changed since you last looked')).toBeAttached();
    await expect(log.locator('.room-notice-change')).toHaveText([/Grace pinned a message/u]);
    await expectNoHorizontalOverflow(page);
    const scan = await new AxeBuilder({ page })
      .include('.room-pins')
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(scan.violations.map(({ id }) => id)).toEqual([]);

    await toggle.click();
    await expect(pins.getByRole('button', { name: /Hide pinned messages/u })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    await expect(pins.getByText('Changed since you last looked')).toHaveCount(0);
    const items = pins.getByRole('listitem');
    await expect(items).toHaveCount(2);
    await expect(items.first()).toContainText(
      'Review milestone 79: implementation and verification details.',
    );
    // 很早以前那一句还没加载：点一下在对话里往前补，补到了跳过去。
    await expect(items.nth(1)).toContainText('Not loaded yet');
    await expectNoHorizontalOverflow(page);
    const expanded = await new AxeBuilder({ page })
      .include('.room-pins')
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(expanded.violations.map(({ id }) => id)).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath(`room-pins-${String(width)}.png`) });

    await items.nth(1).getByRole('button').click();
    const focused = log.locator('[data-focused="true"]');
    await expect(focused).toHaveCount(1);
    await expect(focused).toContainText('Review milestone 11: implementation and verification');
    await expect(focused).toBeInViewport();
    // 补到了以后列表里也写出那一句。
    await expect(items.nth(1)).toContainText('Review milestone 11');

    // 点最新那一句跳过去；滚开以后再点同一句，还跳回去。
    await items.first().getByRole('button').click();
    await expect(focused).toContainText(
      'Review milestone 79: implementation and verification details.',
    );
    await expect(focused).toBeInViewport();
    await log.evaluate((element) => {
      element.scrollTop = 0;
    });
    await expect(focused).not.toBeInViewport();
    await items.first().getByRole('button').click();
    await expect(focused).toBeInViewport();

    // 对话里那一行“Grace 置顶了一句”也能点过去。
    await log.evaluate((element) => {
      element.scrollTop = 0;
    });
    await log
      .locator('.room-notice-change')
      .getByRole('button', { name: 'See the message' })
      .click();
    await expect(focused).toContainText(
      'Review milestone 79: implementation and verification details.',
    );
    await expect(focused).toBeInViewport();

    // 看过的记在个人工作区：重新打开不再亮。
    await page.reload();
    await expect(page.getByRole('region', { name: 'Pinned messages' })).toBeVisible();
    await expect(page.getByText('Changed since you last looked')).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}
