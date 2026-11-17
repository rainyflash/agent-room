import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openRoomSettings, roomAction } from './support/workspace-navigation';

/**
 * 房间设置“公开”一节里的“GitHub 仓库”一组和房间名旁边的勾（specs/redesign/project-rooms.md，3P-5），房间
 * “分享”里的“README 徽章”（3P-8b-1）。夹具 `?private&listed&repository=none|pending|bound` 给出还没绑、在等
 * 核对、绑上了三种样子；在等的认领由夹具的 `claims()` 给（服务器上列自己在等的认领），`&asAdmin` 是房间里
 * 不是房主的人。夹具的 `binding()` 不管是谁都回绑定，所以不是房主时看不到徽章靠的是网页自己不去问。
 */
const roomFixture = '/e2e/fixtures/lobby-scene.html?private&listed';

async function expectAccessible(page: Page, selector: string): Promise<void> {
  const scan = await new AxeBuilder({ page })
    .include(selector)
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({ id, targets: nodes.map(({ target }) => target) })),
  ).toEqual([]);
}

async function openShare(page: Page) {
  await (await roomAction(page, 'Share')).click();
  const dialog = page.getByRole('dialog', { name: 'Share this room' });
  await expect(dialog.getByRole('textbox', { name: 'Link' })).toBeVisible();
  return dialog;
}

async function repositoryGroup(page: Page, state: 'none' | 'pending' | 'bound', extra = '') {
  await page.goto(`${roomFixture}&repository=${state}${extra}`);
  const dialog = await openRoomSettings(page, 'Public');
  const group = dialog.getByRole('region', { name: 'GitHub repository' });
  await group.scrollIntoViewIfNeeded();
  return { dialog, group };
}

for (const width of [1440, 390]) {
  test.describe(`GitHub 仓库：${String(width)}`, () => {
    test.beforeEach(async ({ page }) => {
      await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    });

    test('还没绑：填仓库点“绑上”，给出徽章和 AGENTS.md 那一行', async ({ page }, testInfo) => {
      const failures = collectPageFailures(page);
      const { group } = await repositoryGroup(page, 'none');

      await expectAccessible(page, '.room-repository');
      await group.getByRole('textbox', { name: 'Repository' }).fill('rainyflash/agent-room');
      await group.getByRole('button', { name: 'Link' }).click();

      await expect(group).toContainText('Waiting for rainyflash/agent-room');
      await expect(group.getByLabel('README badge')).toContainText(
        'https://agentroom.test/gh/rainyflash/agent-room/badge.svg',
      );
      await expect(group.getByLabel('Line for AGENTS.md')).toContainText(
        'agent-room join https://agentroom.test/r/aB3xYz',
      );
      await expect(group.getByRole('button', { name: 'Copy' })).toHaveCount(2);
      await expect(group).toContainText(
        'It checks by itself after you push, or you can check now.',
      );
      await expectNoHorizontalOverflow(page);
      await expectAccessible(page, '.room-repository');
      await page.screenshot({
        path: testInfo.outputPath(`repository-claimed-${String(width)}.png`),
        fullPage: true,
      });
      expect(failures).toEqual([]);
    });

    test('在等：上次核对的结果，“现在核对”一分钟一次，“不绑了”回到输入框', async ({
      page,
    }, testInfo) => {
      const failures = collectPageFailures(page);
      const { group } = await repositoryGroup(page, 'pending');

      await expect(group).toContainText('Waiting for rainyflash/agent-room');
      await expect(group).toContainText('the line isn’t in the repository yet');
      await expectAccessible(page, '.room-repository');
      await page.screenshot({
        path: testInfo.outputPath(`repository-pending-${String(width)}.png`),
        fullPage: true,
      });

      const checkNow = group.getByRole('button', { name: 'Check now' });
      await checkNow.click();
      await expect(checkNow).toBeEnabled();
      await checkNow.click();
      await expect(
        group.getByText(/^You can check once a minute\. Try again in \d+ seconds?\.$/u),
      ).toBeVisible();
      await expect(checkNow).toBeDisabled();
      await expectNoHorizontalOverflow(page);

      await group.getByRole('button', { name: 'Stop linking' }).click();
      await expect(group.getByRole('textbox', { name: 'Repository' })).toBeVisible();
      expect(failures).toEqual([]);
    });

    test('绑上了：谁什么时候在哪个文件里核对上，不能换分享码，房间名旁边一个勾', async ({
      page,
    }, testInfo) => {
      const failures = collectPageFailures(page);
      const { dialog, group } = await repositoryGroup(page, 'bound');

      const repository = group.getByRole('link', {
        name: 'Open rainyflash/agent-room on GitHub (opens in a new tab)',
      });
      await expect(repository).toHaveAttribute('href', 'https://github.com/rainyflash/agent-room');
      await expect(group).toContainText(/Verified by you on .+, found in AGENTS\.md\./u);
      await expect(dialog.getByRole('button', { name: 'New share code' })).toHaveCount(0);
      await expect(dialog).toContainText(
        'A room linked to a repository can’t change its share link: the link is written in the repository.',
      );
      await group.getByRole('button', { name: 'Check again' }).click();
      await expect(group).toContainText('the link is still in AGENTS.md.');
      await expectNoHorizontalOverflow(page);
      await expectAccessible(page, '.room-repository');
      await page.screenshot({
        path: testInfo.outputPath(`repository-bound-${String(width)}.png`),
        fullPage: true,
      });

      await page.keyboard.press('Escape');
      await expect(dialog).toBeHidden();
      const mark = page.getByRole('link', {
        name: 'Verified GitHub repository rainyflash/agent-room',
      });
      await expect(mark).toBeVisible();
      await expect(mark).toHaveAttribute('href', 'https://github.com/rainyflash/agent-room');
      await expect(mark).toHaveAttribute('target', '_blank');
      await expect(mark).toHaveAttribute(
        'title',
        'Verified GitHub repository rainyflash/agent-room',
      );
      await expectNoHorizontalOverflow(page);
      await page.screenshot({
        path: testInfo.outputPath(`repository-mark-${String(width)}.png`),
      });
      expect(failures).toEqual([]);
    });

    test('房间“分享”：绑着仓库时房主多“README 徽章”一节，复制的是服务器给的那一行', async ({
      context,
      page,
    }, testInfo) => {
      const failures = collectPageFailures(page);
      await context.grantPermissions(['clipboard-read', 'clipboard-write']);
      await page.goto(`${roomFixture}&repository=bound`);
      const dialog = await openShare(page);

      const badge = dialog.getByRole('region', { name: 'README badge' });
      await badge.scrollIntoViewIfNeeded();
      await expect(badge).toContainText(
        'Put it in the repository’s README. The badge shows how many are online here, and clicking it opens the link above.',
      );
      const markdown =
        '[![Agent Room](https://agentroom.test/gh/rainyflash/agent-room/badge.svg)](https://agentroom.test/r/aB3xYz)';
      await expect(badge.getByLabel('README badge')).toHaveText(markdown);
      await badge.getByRole('button', { name: 'Copy badge' }).click();
      await expect(badge.getByRole('button', { name: 'Copied' })).toBeVisible();
      expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(markdown);
      await expectNoHorizontalOverflow(page);
      await expectAccessible(page, '.ar-dialog');
      await page.screenshot({
        path: testInfo.outputPath(`share-readme-badge-${String(width)}.png`),
        fullPage: true,
      });
      expect(failures).toEqual([]);
    });
  });
}

test('房间“分享”里的“README 徽章”：没绑、在等核对、不是房主都没有', async ({ page }) => {
  for (const query of ['&repository=none', '&repository=pending', '&repository=bound&asAdmin']) {
    await page.goto(`${roomFixture}${query}`);
    // 不是房主时房间照样绑着（勾在），只是网页不去问房主才问得到的那一份。
    await expect(
      page.getByRole('link', { name: 'Verified GitHub repository rainyflash/agent-room' }),
    ).toHaveCount(query.includes('bound') ? 1 : 0);
    const dialog = await openShare(page);
    await expect(dialog.getByRole('heading', { name: 'How it looks when posted' })).toBeVisible();
    await expect(dialog.getByRole('region', { name: 'README badge' })).toHaveCount(0);
    await expect(dialog.getByRole('button', { name: 'Copy badge' })).toHaveCount(0);
  }
});

test('没绑仓库的房间顶栏没有勾；不是房主看不到“GitHub 仓库”一组', async ({ page }) => {
  await page.goto(`${roomFixture}&repository=none&asAdmin`);
  await expect(page.getByRole('link', { name: /^Verified GitHub repository/u })).toHaveCount(0);
  const dialog = await openRoomSettings(page, 'Public');
  await expect(dialog).toContainText('Only the owner can change these settings.');
  await expect(dialog.getByRole('region', { name: 'GitHub repository' })).toHaveCount(0);
});

test('不是房主也看得到勾；“GitHub 仓库”一组照旧只给房主', async ({ page }) => {
  await page.goto(`${roomFixture}&repository=bound&asAdmin`);
  const mark = page.getByRole('link', {
    name: 'Verified GitHub repository rainyflash/agent-room',
  });
  await expect(mark).toBeVisible();
  await expect(mark).toHaveAttribute('href', 'https://github.com/rainyflash/agent-room');
  const dialog = await openRoomSettings(page, 'Public');
  await expect(dialog.getByRole('region', { name: 'GitHub repository' })).toHaveCount(0);
});

test('解绑超名额：说不绑仓库的公开房间每个账户只能有一间', async ({ page }) => {
  const { group } = await repositoryGroup(page, 'bound', '&repositoryLimit');
  await group.getByRole('button', { name: 'Unlink' }).click();
  await group.getByRole('button', { name: 'Unlink' }).click();
  await expect(group).toContainText(
    'Each account can have only one public room that isn’t linked to a repository.',
  );
  await expect(group.getByRole('link', { name: /^Open rainyflash\/agent-room/u })).toBeVisible();
});
