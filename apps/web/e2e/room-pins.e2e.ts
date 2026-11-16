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

    await items.nth(1).locator('.room-pins__item').click();
    const focused = log.locator('[data-focused="true"]');
    await expect(focused).toHaveCount(1);
    await expect(focused).toContainText('Review milestone 11: implementation and verification');
    await expect(focused).toBeInViewport();
    // 补到了以后列表里也写出那一句。
    await expect(items.nth(1)).toContainText('Review milestone 11');

    // 点最新那一句跳过去；滚开以后再点同一句，还跳回去。
    await items.first().locator('.room-pins__item').click();
    await expect(focused).toContainText(
      'Review milestone 79: implementation and verification details.',
    );
    await expect(focused).toBeInViewport();
    await log.evaluate((element) => {
      element.scrollTop = 0;
    });
    await expect(focused).not.toBeInViewport();
    await items.first().locator('.room-pins__item').click();
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

test('置顶：房主在对话里置顶一句、在列表里取消；满了照实说，能打开置顶去取消', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  // `unfolded`：Agent 之间的话不折起来，每一句都摆出来。
  await page.goto(`${roomFixture}?private&pins&history&unfolded&view=conversation`);

  const pins = page.getByRole('region', { name: 'Pinned messages' });
  const log = page.getByRole('log', { name: /conversation/iu });
  const count = pins.locator('.room-pins__count');
  const said = (milestone: number) =>
    log
      .locator('[data-conversation-message-id]')
      .filter({ hasText: `Review milestone ${String(milestone)}: implementation` });
  const pin = async (milestone: number) => {
    const message = said(milestone);
    await message.hover();
    await message.getByRole('button', { name: 'Pin this message' }).click();
  };
  await expect(count).toHaveText('2');

  await pin(78);
  await expect(count).toHaveText('3');
  await expect(pins.locator('.room-pins__summary')).toContainText('Review milestone 78');
  await expect(log.locator('.room-notice-change').last()).toContainText('You pinned a message');
  await expect(said(78).getByRole('button', { name: 'Unpin this message' })).toBeAttached();
  // 自己改的不亮点。
  await expect(pins.getByText('Changed since you last looked')).toHaveCount(0);

  await pin(77);
  await pin(76);
  await expect(count).toHaveText('5');
  // 满了：先在这一行下面说，不去改。
  await pin(75);
  const alert = pins.getByRole('alert');
  await expect(alert).toHaveText(/A room can have up to 5 pinned messages\. Unpin one first\./u);
  await expect(count).toHaveText('5');
  await expectNoHorizontalOverflow(page);

  await alert.getByRole('button', { name: 'Open pinned' }).click();
  const items = pins.getByRole('listitem');
  await expect(items).toHaveCount(5);
  await expect(items.first()).toContainText('Review milestone 76');
  const scan = await new AxeBuilder({ page })
    .include('.room-pins')
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(scan.violations.map(({ id }) => id)).toEqual([]);
  await items.first().getByRole('button', { name: 'Unpin this message' }).click();
  await expect(count).toHaveText('4');
  await expect(log.locator('.room-notice-change').last()).toContainText('You unpinned a message');
  // 又改了一次，上一次没成的那句话就收起来了。
  await expect(pins.getByRole('alert')).toHaveCount(0);

  // 这下置顶得进去了。
  await pin(75);
  await expect(count).toHaveText('5');
  expect(failures).toEqual([]);
});

test('置顶：控制面说不行时照实说，能关掉', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(
    `${roomFixture}?private&history&unfolded&pinsFail=room_pins.rate_limited&view=conversation`,
  );
  const log = page.getByRole('log', { name: /conversation/iu });
  const message = log
    .locator('[data-conversation-message-id]')
    .filter({ hasText: 'Review milestone 79: implementation' });
  await message.hover();
  await message.getByRole('button', { name: 'Pin this message' }).click();
  // 还没有置顶时这一行只为说这句话出来。
  const pins = page.getByRole('region', { name: 'Pinned messages' });
  await expect(pins.getByRole('alert')).toHaveText(
    /The brief and pins were changed too often in the last hour\. Try again later\./u,
  );
  await expect(pins.locator('.room-pins__count')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  await pins.getByRole('button', { name: 'Dismiss' }).click();
  await expect(page.getByRole('region', { name: 'Pinned messages' })).toHaveCount(0);
});
