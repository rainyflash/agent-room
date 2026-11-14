import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { openRoomSettings } from './support/workspace-navigation';

const roomFixture = '/e2e/fixtures/lobby-scene.html';

async function expectAccessible(page: Page, selector: string): Promise<void> {
  const scan = await new AxeBuilder({ page })
    .include(selector)
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(
    scan.violations.map(({ id, nodes }) => ({ id, targets: nodes.map(({ target }) => target) })),
  ).toEqual([]);
}

for (const width of [1440, 390]) {
  test(`房间设置的“公开”：私密设成公开先确认，拿到分享链接，换码，改回私密：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private`);

    const dialog = await openRoomSettings(page, 'Public');
    const visibility = dialog.getByRole('group', { name: 'Who can find this room' });
    await expect(visibility.getByRole('radio', { name: /^Private/u })).toBeChecked();
    await expect(dialog.getByRole('group', { name: 'How people join' })).toHaveCount(0);

    await visibility.getByRole('radio', { name: /^Public/u }).check();
    await dialog.getByRole('radio', { name: /^Ask to join/u }).check();
    await expect(dialog.getByRole('checkbox', { name: /^Manage/u })).toBeDisabled();
    await dialog.getByRole('button', { name: 'Save' }).click();
    await expect(dialog).toContainText('Make this room public?');
    await expect(dialog).toContainText('It stays encrypted');
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page, '.ar-dialog');
    await page.screenshot({
      path: testInfo.outputPath(`room-listing-confirm-${String(width)}.png`),
    });

    await dialog.getByRole('button', { name: 'Make public' }).click();
    await expect(dialog).toContainText('Saved.');
    await expect(dialog.getByLabel('Share link')).toHaveText('https://agentroom.test/r/aB3xYz');

    await dialog.getByRole('button', { name: 'New share code' }).click();
    await dialog.getByRole('button', { name: 'Change it' }).click();
    await expect(dialog.getByLabel('Share link')).toHaveText('https://agentroom.test/r/Q7mPz2');

    await visibility.getByRole('radio', { name: /^Private/u }).check();
    await dialog.getByRole('button', { name: 'Save' }).click();
    await expect(dialog).toContainText('Make this room private?');
    await dialog.getByRole('button', { name: 'Make private' }).click();
    await expect(dialog).toContainText('Saved.');
    await expect(dialog.getByLabel('Share link')).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}

test('房间设置的“公开”：公开着的不加密房间写清楚谁读得到', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${roomFixture}?private&listed`);

  const dialog = await openRoomSettings(page, 'Public');
  await expect(dialog).toContainText('This room is not encrypted');
  await expect(dialog.getByLabel('Share link')).toHaveText('https://agentroom.test/r/aB3xYz');
  await dialog.getByRole('radio', { name: /^Invite only/u }).check();
  await dialog.getByRole('button', { name: 'Save' }).click();
  await expect(dialog).toContainText('Invite-only rooms have no share link');
  expect(failures).toEqual([]);
});

for (const width of [1440, 390]) {
  test(`公开房间有人申请：齿轮上亮点，“成员”一节里批准、拒绝，设管理员：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private&listed&requests`);

    const gear = page.getByRole('button', { name: 'Room settings', exact: true });
    await expect(gear).toHaveAccessibleDescription('Requests to join: 2');
    await expect(gear.locator('.room-desk__attention')).toBeVisible();
    await page.locator('.room-desk__top').screenshot({
      path: testInfo.outputPath(`room-join-request-dot-${String(width)}.png`),
    });

    const dialog = await openRoomSettings(page, 'Members');
    await expect(dialog).toContainText('Public · Ask to join');
    const requests = dialog.locator('.private-room-join-requests');
    await expect(requests.getByRole('heading', { name: 'Requests to join' })).toBeVisible();
    await expect(requests).toContainText('Can I bring my agent?');
    await requests.scrollIntoViewIfNeeded();
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page, '.ar-dialog');
    await page.screenshot({
      path: testInfo.outputPath(`room-join-requests-${String(width)}.png`),
    });

    await requests.getByRole('button', { name: 'Approve Lin' }).click();
    await expect(requests).not.toContainText('Lin');
    await requests.getByRole('button', { name: 'Reject Mo' }).click();
    await expect(requests).toContainText('No one is waiting.');

    // Grace 原来只能查看和发言：设成管理员以后写着“管理员”，能撤回去。
    await dialog.getByRole('button', { name: 'Make admin' }).click();
    const grace = dialog.locator('.private-room-members > ol > li', { hasText: 'Grace' });
    await expect(grace.locator('.private-room-member__role')).toHaveText('Admin');
    await grace.scrollIntoViewIfNeeded();
    await page.screenshot({
      path: testInfo.outputPath(`room-admin-${String(width)}.png`),
    });
    await grace.getByRole('button', { name: 'Remove admin' }).click();
    await expect(grace.locator('.private-room-member__role')).toHaveCount(0);
    expect(failures).toEqual([]);

    await dialog.getByRole('button', { name: 'Close room settings' }).click();
    await expect(gear).not.toHaveAccessibleDescription('Requests to join: 2');
    await expect(gear.locator('.room-desk__attention')).toHaveCount(0);
  });
}

test('公开房间里请网络 Agent：直接进的拿房间号就进，改成只能邀请以后只给口令', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1440 });
  await page.goto(`${roomFixture}?private&listed`);
  const openNetworkInvite = async () => {
    await page
      .locator('.room-desk__top')
      .getByRole('button', { name: 'Bring an agent', exact: true })
      .click();
    const invite = page.getByRole('dialog', { name: 'Bring an agent' });
    await invite.getByRole('radio', { name: /^Network/u }).click();
    return invite;
  };

  let invite = await openNetworkInvite();
  await expect(invite.getByText(/is a public room anyone can join/u)).toBeVisible();
  const message = invite.getByLabel('Message for your agent');
  await expect(message).toContainText(
    'enter the Agent Room public room “Builders Exchange” with its room number 01990d9e-8400-7000-8000-000000000401',
  );
  await expect(invite.getByRole('button', { name: /code and copy/u })).toHaveCount(0);
  await expect(invite.getByText(/the server can read/u)).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
  await invite.getByRole('button', { name: 'Close', exact: true }).click();

  const settings = await openRoomSettings(page, 'Public');
  await settings.getByRole('radio', { name: /^Invite only/u }).check();
  await settings.getByRole('button', { name: 'Save' }).click();
  await expect(settings).toContainText('Invite-only rooms have no share link');
  await settings.getByRole('button', { name: 'Close room settings' }).click();

  invite = await openNetworkInvite();
  await expect(
    invite.getByText(/only lets network agents in with a code: knocking with the room number/u),
  ).toBeVisible();
  await invite.getByRole('button', { name: 'Create code and copy' }).click();
  await expect(invite.getByLabel('Message for your agent')).toContainText(
    'enter the Agent Room public room “Builders Exchange” with the code',
  );
  await expect(invite.getByLabel('Message for your agent')).not.toContainText('knock');
  await expect(invite.getByText('Rather not use a code? Let it knock')).toHaveCount(0);
  expect(failures).toEqual([]);
});

test('不加密的公开房间：房间顶上一直写着公开、不加密', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(`${roomFixture}?private&listed`);
  const identity = page.locator('.room-desk__meta');
  await expect(identity).toContainText('Public room · Not encrypted');
  await expect(identity.locator('.workspace-header__notice')).toHaveAttribute(
    'title',
    'This public room is not encrypted: everyone who joins can read what’s said here, and so can the server.',
  );
  expect(failures).toEqual([]);
});

for (const width of [1440, 390]) {
  test(`新建房间能选公开：先说不加密，再选别人怎么进：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?features=1`);
    await page.getByRole('button', { name: 'New room', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New room' });
    const visibility = dialog.getByRole('group', { name: 'Who can find this room' });
    await expect(visibility.getByRole('radio', { name: /^Private/u })).toBeChecked();
    await expect(dialog.getByRole('group', { name: 'How people join' })).toHaveCount(0);

    await visibility.getByRole('radio', { name: /^Public/u }).check();
    await expect(dialog).toContainText('Public rooms are not encrypted');
    await expect(dialog.getByRole('radio', { name: /^Join directly/u })).toBeChecked();
    await dialog.getByRole('radio', { name: /^Ask to join/u }).check();
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page, '.ar-dialog');
    await page.screenshot({
      path: testInfo.outputPath(`new-public-room-${String(width)}.png`),
    });
    expect(failures).toEqual([]);
  });
}

for (const width of [1440, 390]) {
  test(`公开房间的“分享”：链接、能改的话、三家的分享网址和二维码：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private&listed`);
    await page.getByRole('button', { name: 'Share', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'Share this room' });
    await expect(dialog.getByRole('textbox', { name: 'Link' })).toHaveValue(
      'https://agentroom.test/r/aB3xYz',
    );
    const message = dialog.getByRole('textbox', { name: 'What you’ll say' });
    await expect(message).toHaveValue(/^Come chat in “[^”]+”/u);
    await message.fill('Come say hi! https://agentroom.test/r/aB3xYz');
    const x = new URL(
      (await dialog
        .getByRole('link', { name: 'Post to X (opens in a new tab)' })
        .getAttribute('href')) ?? '',
    );
    expect(x.searchParams.get('text')).toBe('Come say hi!');
    expect(x.searchParams.get('url')).toBe('https://agentroom.test/r/aB3xYz');
    await expect(dialog.getByRole('img', { name: /^QR code for the link to / })).toBeVisible();
    await expect(dialog.getByRole('heading', { name: 'How it looks when posted' })).toBeVisible();
    // 预览图真的加载出来了：文件名和 `public/` 里的对不上时这里是 0。
    await expect(dialog.locator('.room-share__preview img')).toHaveJSProperty(
      'naturalWidth',
      1_280,
    );
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page, '.ar-dialog');
    await page.screenshot({ path: testInfo.outputPath(`share-${String(width)}.png`) });
    await page.keyboard.press('Escape');
    await expect(dialog).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}

test('“分享”只给公开房间：私密的没有', async ({ page }) => {
  await page.goto(`${roomFixture}?private`);
  await expect(page.getByRole('button', { name: 'Invite people' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Share', exact: true })).toHaveCount(0);
});
