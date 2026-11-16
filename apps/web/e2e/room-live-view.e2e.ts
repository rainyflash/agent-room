import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openRoomSettings } from './support/workspace-navigation';

/**
 * 房间设置“公开”一节里的“分享页放实况”一组和房间名旁边那一小句（specs/redesign/project-rooms.md，3P-6b）。
 * 夹具 `?private&listed&live=on|off|ineligible` 给出开着、关着、开不了（房间要申请才能进）三种样子；和服务器
 * 一样，公开设置换掉“直接进”时实况自己关掉。`&asAdmin` 是房间里不是房主的管理员。
 */
const roomFixture = '/e2e/fixtures/lobby-scene.html?private&listed';
const switchName = 'Show what’s said here on the share page';
const liveLine = 'Visible without signing in';
const liveTitle =
  'This public room shows a live view on its share page: people who aren’t signed in can see what’s said here. It isn’t encrypted, so the server can read it too.';

async function expectAccessible(page: Page, selector: string): Promise<void> {
  const scan = await new AxeBuilder({ page })
    .include(selector)
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({ id, targets: nodes.map(({ target }) => target) })),
  ).toEqual([]);
}

/** 顶栏房间名旁边那一小句（窄的时候只剩后一半，宽的时候前面还有“Public room ·”）。 */
function topBarLine(page: Page) {
  return page.locator('.workspace-header__notice').filter({ visible: true });
}

async function openLiveGroup(page: Page) {
  const dialog = await openRoomSettings(page, 'Public');
  const group = dialog.getByRole('region', { name: 'Live view on the share page' });
  await group.scrollIntoViewIfNeeded();
  return { dialog, group };
}

for (const width of [1440, 390]) {
  test.describe(`分享页放实况：${String(width)}`, () => {
    test.beforeEach(async ({ page }) => {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    });

    test('关着：打开先确认，说清楚不登录的人看得到什么；开好以后顶栏多那一小句', async ({
      page,
    }, testInfo) => {
      const failures = collectPageFailures(page);
      await page.goto(`${roomFixture}&live=off`);
      await expect(page.getByRole('heading', { level: 1 }).first()).toBeVisible();
      await expect(page.getByText(liveLine)).toHaveCount(0);

      const { dialog, group } = await openLiveGroup(page);
      await expect(group).toContainText(
        'When it’s on, people who aren’t signed in can see what’s said here from then on, on the share page.',
      );
      const toggle = group.getByRole('switch', { name: switchName });
      await expect(toggle).not.toBeChecked();
      await expect(toggle).toHaveAccessibleDescription('Off.');
      await expectAccessible(page, '.room-live-view');

      await toggle.click();
      const confirmation = group.getByRole('status').filter({
        hasText: 'Show what’s said here on the share page?',
      });
      await expect(confirmation).toContainText(
        'From now on, anyone can see what’s said in this room on its share page without signing in',
      );
      await expectNoHorizontalOverflow(page);
      await expectAccessible(page, '.room-live-view');
      await page.screenshot({
        path: testInfo.outputPath(`live-view-confirm-${String(width)}.png`),
        fullPage: true,
      });

      await group.getByRole('button', { name: 'Turn on' }).click();
      await expect(toggle).toHaveAccessibleDescription(
        /^On since .+\. Only what’s said after that is shown\.$/u,
      );
      await expect(toggle).toBeChecked();
      await expect(group.getByRole('button', { name: 'Turn on' })).toHaveCount(0);

      await page.keyboard.press('Escape');
      await expect(dialog).toBeHidden();
      await expect(topBarLine(page)).toContainText(liveLine);
      await expect(topBarLine(page)).toHaveAttribute('title', liveTitle);
      await expectNoHorizontalOverflow(page);
      await page.screenshot({
        path: testInfo.outputPath(`live-view-top-bar-${String(width)}.png`),
      });
      expect(failures).toEqual([]);
    });

    test('开着：顶栏一直说不登录也看得到；关掉不用确认，那一小句跟着没了', async ({
      page,
    }, testInfo) => {
      const failures = collectPageFailures(page);
      await page.goto(`${roomFixture}&live=on`);
      await expect(topBarLine(page)).toContainText(liveLine);
      await expect(topBarLine(page)).toHaveAttribute('title', liveTitle);

      const { dialog, group } = await openLiveGroup(page);
      const toggle = group.getByRole('switch', { name: switchName });
      await expect(toggle).toBeChecked();
      await expectAccessible(page, '.room-live-view');
      await page.screenshot({
        path: testInfo.outputPath(`live-view-on-${String(width)}.png`),
        fullPage: true,
      });

      await toggle.click();
      await expect(toggle).not.toBeChecked();
      await expect(toggle).toHaveAccessibleDescription('Off.');
      await expect(group.getByRole('button', { name: 'Turn on' })).toHaveCount(0);

      await page.keyboard.press('Escape');
      await expect(dialog).toBeHidden();
      await expect(page.getByText(liveLine)).toHaveCount(0);
      expect(failures).toEqual([]);
    });

    test('开不了：开关灰着，说要先改成直接进；改成直接进存好以后就能开', async ({
      page,
    }, testInfo) => {
      const failures = collectPageFailures(page);
      await page.goto(`${roomFixture}&live=ineligible`);
      const { dialog, group } = await openLiveGroup(page);

      const toggle = group.getByRole('switch', { name: switchName });
      await expect(toggle).toBeDisabled();
      await expect(group.getByRole('listitem')).toHaveText(
        'Only rooms people can join directly can show a live view. Set How people join to Join directly first.',
      );
      await expectNoHorizontalOverflow(page);
      await expectAccessible(page, '.room-live-view');
      await page.screenshot({
        path: testInfo.outputPath(`live-view-ineligible-${String(width)}.png`),
        fullPage: true,
      });

      await dialog.getByRole('radio', { name: /^Join directly/u }).click();
      await dialog.getByRole('button', { name: 'Save' }).click();
      await expect(dialog.getByText('Saved.')).toBeVisible();
      await expect(toggle).toBeEnabled();
      await expect(group.getByRole('listitem')).toHaveCount(0);
      expect(failures).toEqual([]);
    });
  });
}

test('开着时改成要申请：存好以后实况自己关了，顶栏那一小句也没了', async ({ page }) => {
  await page.goto(`${roomFixture}&live=on`);
  await expect(topBarLine(page)).toContainText(liveLine);
  const { dialog, group } = await openLiveGroup(page);
  await expect(group.getByRole('switch', { name: switchName })).toBeChecked();

  await dialog.getByRole('radio', { name: /^Ask to join/u }).click();
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog.getByText('Saved.')).toBeVisible();
  const toggle = group.getByRole('switch', { name: switchName });
  await expect(toggle).not.toBeChecked();
  await expect(toggle).toBeDisabled();
  await expect(group.getByRole('listitem')).toHaveText(
    'Only rooms people can join directly can show a live view. Set How people join to Join directly first.',
  );

  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(page.getByText(liveLine)).toHaveCount(0);
});

test('不是房主的管理员看得到这一组、也能开关；公开设置照旧只有房主改', async ({ page }) => {
  await page.goto(`${roomFixture}&live=off&asAdmin`);
  const { dialog, group } = await openLiveGroup(page);
  await expect(dialog).toContainText('Only the owner can change these settings.');
  const toggle = group.getByRole('switch', { name: switchName });
  await toggle.click();
  await group.getByRole('button', { name: 'Turn on' }).click();
  await expect(toggle).toBeChecked();
  await expectAccessible(page, '.room-live-view');
});

test('私密的房间没有这一组，顶栏也没有那一小句', async ({ page }) => {
  await page.goto('/e2e/fixtures/lobby-scene.html?private&live=off');
  await expect(page.getByRole('heading', { level: 1 }).first()).toBeVisible();
  await expect(page.getByText(liveLine)).toHaveCount(0);
  const dialog = await openRoomSettings(page, 'Public');
  await expect(dialog.getByRole('group', { name: 'Who can find this room' })).toBeVisible();
  await expect(dialog.getByRole('region', { name: 'Live view on the share page' })).toHaveCount(0);
});
