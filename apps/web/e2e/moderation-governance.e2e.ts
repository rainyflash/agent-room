import { openRoomSettings } from './support/workspace-navigation';
import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html?view=resources';

test('举报不读取受保护正文且房间治理动作可撤销', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  await page
    .getByRole('list', { name: 'Room resources' })
    .getByRole('button', { name: /Protocol review ready/u })
    .click();
  await page.getByRole('button', { name: 'Report' }).click();
  const report = page.getByRole('dialog', { name: 'Report this' });
  await report.getByLabel('Context for the reviewer').fill('Explicit browser evidence only');
  await report.getByRole('checkbox', { name: /Include the summary you can see/iu }).check();
  expect(await fixtureContentReads(page)).toEqual({ downloads: 0, tickets: 0 });
  await report.getByRole('button', { name: 'Send report' }).click();
  await expect(report.getByText('Report sent')).toBeVisible();
  expect(await fixtureContentReads(page)).toEqual({ downloads: 0, tickets: 0 });
  await report.getByRole('button', { name: 'Done' }).click();
  await page.getByRole('button', { name: 'Close message details' }).click();

  const governance = await openRoomSettings(page, 'Moderation');
  await expect(
    governance.getByText('Open only when you want to inspect the verified bytes.'),
  ).toBeVisible();
  const caseSelect = governance.getByLabel('Which report (optional)');
  await caseSelect.selectOption({ index: 1 });
  await governance.getByRole('checkbox', { name: /I understand who this affects/iu }).check();
  await governance.getByRole('button', { name: 'Apply action' }).click();
  await expect(governance.getByRole('img', { name: 'Applied' })).toBeVisible();
  await governance.getByRole('button', { name: 'Undo' }).click();
  await expect(governance.getByRole('img', { name: 'Undone' })).toBeVisible();

  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: '../../artifacts/browser/task-30/moderation-governance.png',
  });
});

type FixtureContentReads = { readonly downloads: number; readonly tickets: number };

async function fixtureContentReads(page: Page): Promise<FixtureContentReads> {
  return await page.evaluate(() => {
    const value: unknown = Reflect.get(window, '__agentRoomFixtureContentReads');
    if (typeof value !== 'object' || value === null) {
      throw new Error('正文读取夹具计数器缺失。');
    }
    const record = value as Record<string, unknown>;
    if (typeof record.downloads !== 'number' || typeof record.tickets !== 'number') {
      throw new Error('正文读取夹具计数器无效。');
    }
    return { downloads: record.downloads, tickets: record.tickets };
  });
}
