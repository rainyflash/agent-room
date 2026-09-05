import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

for (const width of [1440, 390]) {
  test(`对话工作区的视图、草稿、成员与键盘：${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto('/e2e/fixtures/lobby-scene.html');
    await expect(page).toHaveTitle('Agent Room Lobby Fixture');
    await expect(
      page.getByRole('heading', { name: 'Builders Exchange', exact: true }),
    ).toBeVisible();
    await expect(page.locator('canvas')).toHaveCount(0);
    const input = page.getByRole('textbox', { name: 'Message', exact: true });
    await expect(input).toBeInViewport();
    await page.getByRole('button', { name: 'What can you help with?', exact: true }).click();
    await expect(input).toHaveValue('What can you help with?');
    await expect(page.getByRole('log').locator('article')).toHaveCount(0);
    await page.getByRole('tab', { name: 'Resources', exact: true }).click();
    await expect(
      page.getByRole('heading', { name: 'Shared with the room', exact: true }),
    ).toBeVisible();
    await page.getByRole('tab', { name: 'Resources', exact: true }).press('ArrowLeft');
    await expect(page.getByRole('tab', { name: 'Conversation', exact: true })).toBeFocused();
    await expect(input).toHaveValue('What can you help with?');
    await page.getByRole('button', { name: 'View agents', exact: true }).click();
    const members = page.getByRole('dialog', { name: 'Agents in this room', exact: true });
    const search = members.getByRole('searchbox', { name: 'Search agents' });
    await search.fill('Build Agent 003');
    await expect(members.locator('.roster-agent')).toHaveCount(1);
    await members.getByRole('button', { name: 'Close panel', exact: true }).press('Escape');
    await expect(members).not.toBeVisible();
    await expect(page.getByRole('button', { name: 'View agents', exact: true })).toBeFocused();
    await expect(input).toHaveValue('What can you help with?');
    await page.getByRole('button', { name: 'View agents', exact: true }).click();
    await members.getByRole('searchbox', { name: 'Search agents' }).fill('Build Agent 003');
    await members.getByRole('button', { name: /Build Agent 003/u }).click();
    await expect(page.getByRole('complementary')).toContainText('Build Agent 003');
    await expect(page.getByRole('complementary')).toHaveCSS('opacity', '1');
    await expectAccessibleWorkspace(page);
    await page.getByRole('button', { name: 'Close Agent details', exact: true }).click();
    await expect(page.getByRole('complementary')).toHaveCount(0);
    if (width === 390)
      await expect(page.getByRole('button', { name: 'View agents', exact: true })).toBeFocused();
    else
      await expect(
        page.locator('.workspace-members-slot').getByRole('button', { name: /Build Agent 003/u }),
      ).toBeFocused();
    await expectAccessibleWorkspace(page);
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath(`workspace-${String(width)}.png`) });
    await page.getByRole('tab', { name: 'Resources', exact: true }).click();
    await page.reload();
    await expect(page.getByRole('tab', { name: 'Resources', exact: true })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await expectAccessibleWorkspace(page);
  });
}

async function expectAccessibleWorkspace(page: Page): Promise<void> {
  const accessibility = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(accessibility.violations).toEqual([]);
}
