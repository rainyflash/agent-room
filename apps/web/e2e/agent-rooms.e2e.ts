import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

// “我的 Agent”里 Agent 在哪些房间（specs/agent-identity/design.md“网页”）：夹具 Agent 在公共大厅
// Builders Exchange 和私人房间 Weekly review 里；能带它进的还有 Design crit、Board notes（你在那里
// 没有“自动发言”，它进去只能看）和公共大厅 Night Market。
for (const width of [1440, 390]) {
  test(`我的 Agent：${String(width)}px 看它在哪些房间、带进房间、离开`, async ({ page }) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 1000 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/e2e/fixtures/my-agents.html');

    const card = page.getByRole('button', { name: /Studio companion/u });
    await expect(card).toContainText('In 2 rooms');
    await card.click();
    const dialog = page.getByRole('dialog', { name: 'Studio companion' });
    const rooms = dialog.getByRole('region', { name: 'Rooms' });
    await expect(rooms.getByRole('listitem')).toHaveText([/Builders Exchange/u, /Weekly review/u]);

    await rooms.getByRole('button', { name: 'Take into a room' }).click();
    const picker = rooms.getByRole('group', { name: 'Which room?' });
    await expect(picker.getByRole('button')).toHaveText([
      /Design crit/u,
      /Board notes/u,
      /Night Market/u,
    ]);
    await expect(picker.getByRole('button', { name: /Board notes/u })).toContainText(
      'It can only read there',
    );
    await expectNoHorizontalOverflow(page);
    const scan = await new AxeBuilder({ page })
      .include('dialog')
      .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
      .analyze();
    expect(scan.violations.map(({ id }) => id)).toEqual([]);

    await picker.getByRole('button', { name: /Design crit/u }).click();
    await expect(rooms.getByRole('status')).toHaveText(
      'Took it into Design crit. It gets the messages there the next time it connects.',
    );
    await expect(rooms.getByRole('group', { name: 'Which room?' })).toHaveCount(0);
    await expect(rooms.getByRole('listitem')).toHaveCount(3);

    await rooms.getByRole('button', { name: 'Take Studio companion out of Weekly review' }).click();
    await expect(rooms.getByRole('status')).toHaveText('Took it out of Weekly review.');
    await expect(rooms.getByRole('listitem')).toHaveText([/Builders Exchange/u, /Design crit/u]);

    await dialog.getByRole('button', { name: 'Close' }).click();
    await expect(card).toContainText('In 2 rooms');
    expect(failures).toEqual([]);
  });
}

test('它在你的电脑上没有能用的连接时，说清没带进去、怎么办', async ({ page }) => {
  await page.goto('/e2e/fixtures/my-agents.html?bring=none');
  await page.getByRole('button', { name: /Studio companion/u }).click();
  const rooms = page
    .getByRole('dialog', { name: 'Studio companion' })
    .getByRole('region', { name: 'Rooms' });
  await rooms.getByRole('button', { name: 'Take into a room' }).click();
  await rooms.getByRole('button', { name: /Night Market/u }).click();
  await expect(rooms.getByRole('status')).toContainText('It didn’t go in');
  await expect(rooms.getByRole('listitem')).toHaveCount(2);
});

test('中文窄屏：房间一节和房间选择不挤成竖排', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(() => {
    localStorage.setItem('agent-room.language', 'zh-CN');
  });
  await page.goto('/e2e/fixtures/my-agents.html');
  const card = page.getByRole('button', { name: /Studio companion/u });
  await expect(card).toContainText('在 2 个房间');
  await card.click();
  const rooms = page
    .getByRole('dialog', { name: 'Studio companion' })
    .getByRole('region', { name: '在哪些房间' });
  await rooms.getByRole('button', { name: '带进房间' }).click();
  const boardNotes = rooms.getByRole('button', { name: /Board notes/u });
  await expect(boardNotes).toContainText('它进去只能看');
  const box = await boardNotes.boundingBox();
  expect(box?.width ?? 0).toBeGreaterThan(240);
  await expectNoHorizontalOverflow(page);
});
