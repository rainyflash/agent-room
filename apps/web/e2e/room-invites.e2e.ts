import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const roomFixture = '/e2e/fixtures/lobby-scene.html';
const joinFixture = '/e2e/fixtures/join-invite.html';

async function expectAccessible(page: Page, selector: string): Promise<void> {
  const scan = await new AxeBuilder({ page })
    .include(selector)
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({ id, targets: nodes.map(({ target }) => target) })),
  ).toEqual([]);
}

async function linkText(page: Page): Promise<string> {
  const dialog = page.getByRole('dialog', { name: 'Invite people' });
  return (
    (await dialog.locator('text=/https:\\/\\/agentroom\\.test\\/join#/').first().textContent()) ??
    ''
  );
}

for (const width of [1440, 390]) {
  test(`邀请人：生成链接、改权限换一条、停用，按账户 ID 邀请收在折叠里：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private`);

    await page.getByRole('button', { name: 'Invite people', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Invite people' });
    await expect(dialog).toContainText('There is no link yet.');
    await dialog.getByRole('button', { name: 'Make link and copy' }).click();
    await expect(dialog).toContainText('https://agentroom.test/join#');
    await expect(dialog).toContainText('Anyone with the link can come in after signing in');
    const first = await linkText(page);

    // 链接给的权限默认是查看、发言、自动发言；改了以后按新的权限换一条，旧的就不能用了。
    await dialog.getByText('What the link gives').click();
    const permissions = dialog.getByRole('group', { name: 'Link permissions' });
    await expect(permissions.getByRole('checkbox', { name: /^Automate/u })).toBeChecked();
    await permissions.getByRole('checkbox', { name: /^Automate/u }).uncheck();
    await dialog.getByRole('button', { name: 'New link with these' }).click();
    await expect.poll(async () => await linkText(page)).not.toBe(first);
    await expect(dialog.getByRole('button', { name: 'New link with these' })).toHaveCount(0);

    await dialog.getByText('Invite by account ID').click();
    await expect(dialog.getByLabel('Account ID')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page, '.ar-dialog');
    await page.screenshot({ path: testInfo.outputPath(`invite-dialog-${String(width)}.png`) });

    await dialog.getByRole('button', { name: 'Turn off link' }).click();
    await expect(dialog).toContainText('The link is off. Nobody can use it now.');
    await expect(dialog.getByRole('button', { name: 'Make link and copy' })).toBeVisible();
    expect(failures).toEqual([]);
  });
}

test('已经有链接时一打开就看得到；不能邀请人的、归档了的房间没有“邀请人”', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${roomFixture}?private&invite-link`);
  await page.getByRole('button', { name: 'Invite people', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Invite people' })).toContainText(
    'https://agentroom.test/join#',
  );

  // 网络 Agent 建的房间里你是它邀请来的：能查看、发言、自动发言，不能邀请人。
  await page.goto(`${roomFixture}?private&origin=network`);
  await expect(page.getByRole('button', { name: 'Bring an agent', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Invite people', exact: true })).toHaveCount(0);

  // 被邀请的人有自动发言时，接入 Agent 里不多说；只有查看和发言时先说清楚。
  await page.getByRole('button', { name: 'Bring an agent', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Bring an agent' })).not.toContainText(
    'can come in but',
  );
  await page.goto(`${roomFixture}?private&origin=network&speak-only`);
  await page.getByRole('button', { name: 'Bring an agent', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Bring an agent' })).toContainText(
    'agents you bring in can come in but can’t talk',
  );

  await page.goto(`${roomFixture}?private&archived`);
  await expect(page.getByRole('button', { name: 'Find someone', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Invite people', exact: true })).toHaveCount(0);

  // 公共大厅没有邀请。
  await page.goto(roomFixture);
  await expect(page.getByRole('button', { name: 'Bring an agent', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Invite people', exact: true })).toHaveCount(0);
});

for (const width of [1440, 390]) {
  test(`打开邀请链接：看清是谁请的、能做什么，点加入：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(joinFixture);
    await expect(
      page.getByRole('heading', { name: 'Ada invited you to “Weekly review”' }),
    ).toBeVisible();
    await expect(page.getByText('agents you bring in can talk too')).toBeVisible();
    await expect(page.getByText('The link works until')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page, '.entry-card');
    await page.screenshot({ path: testInfo.outputPath(`join-${String(width)}.png`) });

    await page.getByRole('button', { name: 'Join', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Joining…' })).toBeDisabled();
    expect(
      await page.evaluate(
        () => (window as { __agentRoomJoinClicks?: number }).__agentRoomJoinClicks,
      ),
    ).toBe(1);
    expect(failures).toEqual([]);
  });
}

test('打开邀请链接的其它样子：只给发言、已经在房间里、失效、被封禁、试得太多', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${joinFixture}?speak`);
  await expect(page.getByText('Agents you bring in can come in but can’t talk.')).toBeVisible();

  await page.goto(`${joinFixture}?already`);
  await expect(
    page.getByRole('heading', { name: 'You’re already in “Weekly review”' }),
  ).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open room' })).toBeVisible();

  await page.goto(`${joinFixture}?expired`);
  await expect(page.getByRole('heading', { name: 'This link no longer works' })).toBeVisible();
  await expect(page.getByText('Ask the person who invited you for a new one.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Try again' })).toHaveCount(0);
  await expect(page.getByRole('link', { name: 'Back to rooms' })).toHaveAttribute('href', '/rooms');

  await page.goto(`${joinFixture}?banned`);
  await expect(page.getByRole('heading', { name: 'You can’t join this room' })).toBeVisible();

  await page.goto(`${joinFixture}?busy`);
  await expect(page.getByRole('heading', { name: 'Too many tries' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Try again' })).toBeVisible();
});
