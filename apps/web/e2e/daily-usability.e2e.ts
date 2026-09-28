import { expect, test } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`搜索真实分页投影并查看回复话题 ${String(width)}px`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 1000 });
    await page.goto('/e2e/fixtures/lobby-scene.html?view=conversation&history=1');
    await page.getByRole('button', { name: 'Search this conversation', exact: true }).click();
    await expect(page.getByText('20 messages loaded · search covers these messages')).toBeVisible();
    await page.getByLabel('Keyword', { exact: true }).fill('milestone 1:');
    await expect(page.getByText('No messages match these filters.')).toBeVisible();
    for (let i = 0; i < 3; i += 1)
      await page.getByRole('button', { name: 'Load earlier messages', exact: true }).click();
    await expect(page.getByText('80 messages loaded · search covers these messages')).toBeVisible();
    await expect(
      page.getByText('Review milestone 1: implementation and verification details.'),
    ).toBeVisible();
    await page.getByRole('button', { name: 'Clear filters', exact: true }).click();
    const message = page
      .locator('[data-conversation-message-id]')
      .filter({ hasText: 'Review milestone 80:' });
    await message.getByRole('button', { name: 'View this reply topic' }).click();
    await expect(page.locator('[data-conversation-message-id]')).toHaveCount(2);
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath(`history-topic-${String(width)}.png`) });
    expect(failures).toEqual([]);
  });

  test(`收件箱静音已读刷新后保留并定位原消息 ${String(width)}px`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 1000 });
    await page.goto('/e2e/fixtures/lobby-scene.html?inbox=1&history=1');
    await expect(
      page.getByRole('heading', { name: 'Needs your attention', exact: true }),
    ).toBeVisible();
    const entries = page.locator('.personal-inbox__list > li');
    await expect(entries).toHaveCount(4);
    await entries.first().getByRole('button', { name: 'Mute room', exact: true }).click();
    await expect(
      entries.first().getByRole('button', { name: 'Unmute room', exact: true }),
    ).toBeVisible();
    await entries.first().getByRole('button', { name: 'Mark reviewed', exact: true }).click();
    await expect(entries).toHaveCount(3);
    await page.reload();
    await expect(entries).toHaveCount(3);
    await expect(
      entries.first().getByRole('button', { name: 'Unmute room', exact: true }),
    ).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath(`inbox-${String(width)}.png`) });
    const original = entries.first().getByRole('link', { name: 'Open original', exact: true });
    const url = await original.getAttribute('href');
    expect(url).toContain('message=');
    await original.click();
    await expect(page.locator('[data-focused="true"]')).toHaveCount(1);
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
  });

  test(`收藏撤销标签筛选和键盘返回 ${String(width)}px`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 1000 });
    await page.goto('/e2e/fixtures/lobby-scene.html?agents=24');
    await page.getByRole('button', { name: 'List view', exact: true }).click();
    const roster = page.locator('.list-roster:visible');
    await roster.getByRole('button', { name: /Build Agent 001/u }).click();
    const inspector = page.locator('.agent-inspector');
    await inspector.getByRole('button', { name: 'Add to favorites', exact: true }).click();
    await inspector.getByRole('button', { name: 'Undo', exact: true }).click();
    await expect(
      inspector.getByRole('button', { name: 'Add to favorites', exact: true }),
    ).toBeVisible();
    await inspector.getByRole('button', { name: 'Add to favorites', exact: true }).click();
    await inspector.getByLabel('Project tags', { exact: true }).fill('发布验收, Backend');
    await inspector.getByRole('button', { name: 'Save tags', exact: true }).click();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath(`organization-${String(width)}.png`) });
    await page.getByRole('button', { name: 'Close Agent details' }).click();
    await expect(roster.getByRole('button', { name: /Build Agent 001/u })).toBeFocused();
    await roster.getByLabel('Your agent collection').selectOption('favorites');
    await roster.getByLabel('Filter by project tag').selectOption('Backend');
    await expect(roster.locator('.list-roster__list > li')).toHaveCount(1);
    await page.reload();
    await page.getByRole('button', { name: 'List view', exact: true }).click();
    await roster.getByLabel('Your agent collection').selectOption('favorites');
    await roster.getByLabel('Filter by project tag').selectOption('Backend');
    await expect(roster.locator('.list-roster__list > li')).toHaveCount(1);
    expect(failures).toEqual([]);
  });

  test(`加载更早消息和重新打开均保留阅读位置 ${String(width)}px`, async ({ page }) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 1000 });
    await page.goto('/e2e/fixtures/lobby-scene.html?view=conversation&history=1');
    const history = page.locator('.conversation-panel__timeline').first();
    await expect(history.locator('[data-conversation-message-id]')).toHaveCount(20);
    // “加载更早的消息”在时间线顶端：翻到顶再点，原来最早的那条要留在原处。
    await history.evaluate((element) => {
      element.scrollTop = 0;
    });
    const target = history
      .locator('[data-conversation-message-id]')
      .filter({ hasText: 'Review milestone 61:' });
    const before = await target.evaluate((element) => element.getBoundingClientRect().top);
    await history.getByRole('button', { name: 'Load earlier messages', exact: true }).click();
    await expect(history.locator('[data-conversation-message-id]')).toHaveCount(40);
    // Browser scrollTop is rounded to a physical pixel on fractional layouts.
    await expect
      .poll(async () =>
        Math.abs(
          (await target.evaluate((element) => element.getBoundingClientRect().top)) - before,
        ),
      )
      .toBeLessThanOrEqual(1);
    // 把这条读到顶端，刷新后回到同一位置。测试页刷新后只重新加载最近 20 条，这条在其中。
    await target.evaluate((element) => {
      element.scrollIntoView({ block: 'start' });
    });
    const reading = await target.evaluate((element) => element.getBoundingClientRect().top);
    // 阅读位置在滚动停下 300ms 后保存。
    await page.waitForTimeout(500);
    await page.reload();
    await expect(target).toBeVisible();
    await expect
      .poll(async () =>
        Math.abs(
          (await target.evaluate((element) => element.getBoundingClientRect().top)) - reading,
        ),
      )
      .toBeLessThanOrEqual(1);
    expect(failures).toEqual([]);
  });
}

for (const width of [1440, 390]) {
  test(`长消息默认收起，可以展开和收起 ${String(width)}px`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 1000 });
    await page.goto('/e2e/fixtures/lobby-scene.html?view=conversation&history=1&long=1');
    const history = page.locator('.conversation-panel__timeline').first();
    const message = history
      .locator('[data-conversation-message-id]')
      .filter({ hasText: 'Review milestone 80:' });
    const fold = message.locator('.conversation-message__fold');
    const expand = message.getByRole('button', { name: 'Show the whole message', exact: true });
    await expect(expand).toHaveAttribute('aria-expanded', 'false');
    expect(await fold.evaluate((element) => element.getBoundingClientRect().height)).toBeLessThan(
      300,
    );
    await expand.click();
    const collapse = message.getByRole('button', { name: 'Collapse the message', exact: true });
    await expect(collapse).toHaveAttribute('aria-expanded', 'true');
    await expect
      .poll(() => fold.evaluate((element) => element.getBoundingClientRect().height))
      .toBeGreaterThan(400);
    await expect(message).toContainText('Step 30: checked the collection points');
    await collapse.click();
    await expect(expand).toBeVisible();
    // 收起后这条消息回到视野里。
    await expect(message.locator('header')).toBeInViewport();
    // 短消息不出现折叠按钮。
    await expect(
      history
        .locator('[data-conversation-message-id]')
        .filter({ hasText: 'Review milestone 79:' })
        .getByRole('button', { name: 'Show the whole message' }),
    ).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath(`long-message-${String(width)}.png`) });
    expect(failures).toEqual([]);
  });
}

test('对话面板可以放大，刷新后记得', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto('/e2e/fixtures/lobby-scene.html?view=conversation&history=1');
  const panel = page.locator('.room-panel');
  const width = () => panel.evaluate((element) => element.getBoundingClientRect().width);
  const regular = await width();
  expect(regular).toBeGreaterThan(440);
  await page.getByRole('button', { name: 'Widen the conversation', exact: true }).click();
  await expect.poll(width).toBeGreaterThan(regular + 300);
  // 场景让出面板的宽度，面板不盖住场景。
  const scene = await page
    .locator('.room-scene')
    .evaluate((element) => element.getBoundingClientRect().right);
  const left = await panel.evaluate((element) => element.getBoundingClientRect().left);
  expect(scene).toBeLessThanOrEqual(left);
  await page.reload();
  const restore = page.getByRole('button', { name: 'Restore the conversation width', exact: true });
  await expect(restore).toBeVisible();
  await restore.click();
  await expect.poll(width).toBeLessThan(regular + 1);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('一千个人物可逐个定位且能通过收藏缩小列表', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto('/e2e/fixtures/lobby-scene.html?agents=1000');
  const scene = page.getByRole('listbox', { name: 'Interactive Agent room scene' });
  await expect(scene.locator('canvas')).toBeVisible();
  await expect(scene.getByRole('option')).toHaveCount(1000);
  await scene.focus();
  await page.keyboard.press('End');
  await page.keyboard.press('Enter');
  await expect(page.locator('.agent-inspector')).toBeVisible();
  await page.getByRole('button', { name: 'Add to favorites', exact: true }).click();
  await page.getByRole('button', { name: 'Close Agent details' }).click();
  await expect(scene).toBeFocused();
  await page.screenshot({ path: testInfo.outputPath('large-room.png') });
  await page.getByRole('button', { name: 'List view', exact: true }).click();
  await page.getByLabel('Your agent collection').selectOption('favorites');
  await expect(page.locator('.list-roster__list > li')).toHaveCount(1);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});
