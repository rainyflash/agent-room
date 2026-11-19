import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Locator, type Page } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';
import { expectShareCardFor } from './support/share-card';
import { roomAction } from './support/workspace-navigation';
import type { LobbyFixtureWindow } from '../src/test/lobby-fixture-controls';

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

function inviteCard(dialog: Locator): Locator {
  return dialog.getByRole('img', {
    name: 'Invite card for Builders Exchange, with a QR code for the link',
  });
}

/** 邀请卡片画好了，上面的二维码就是这条链接。 */
async function expectCardFor(dialog: Locator, url: string): Promise<void> {
  await expectShareCardFor(inviteCard(dialog), url);
}

for (const width of [1440, 390]) {
  test(`邀请人：生成链接、改权限换一条、停用，按账户 ID 邀请收在折叠里：${String(width)}`, async ({
    page,
  }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: width === 390 ? 844 : 900 });
    await page.goto(`${roomFixture}?private`);

    await (await roomAction(page, 'Invite people')).click();
    const dialog = page.getByRole('dialog', { name: 'Invite people' });
    await expect(dialog).toContainText('There is no link yet.');
    await dialog.getByRole('button', { name: 'Make link and copy' }).click();
    await expect(dialog).toContainText('https://agentroom.test/join#');
    await expect(dialog).toContainText('Anyone with the link can come in after signing in');
    const first = await linkText(page);
    // 有了链接，旁边是一张邀请卡片，一起发的话里带着链接。
    await expectCardFor(dialog, first);
    await expect(dialog.getByLabel('What you’ll say')).toHaveValue(
      `Join me in “Builders Exchange” on Agent Room. Open the link and sign in:\n${first}`,
    );

    // 链接给的权限默认是查看、发言、自动发言；改了以后按新的权限换一条，旧的就不能用了。
    await dialog.getByText('What the link gives').click();
    const permissions = dialog.getByRole('group', { name: 'Link permissions' });
    await expect(permissions.getByRole('checkbox', { name: /^Automate/u })).toBeChecked();
    await permissions.getByRole('checkbox', { name: /^Automate/u }).uncheck();
    await dialog.getByRole('button', { name: 'New link with these' }).click();
    await expect.poll(async () => await linkText(page)).not.toBe(first);
    await expect(dialog.getByRole('button', { name: 'New link with these' })).toHaveCount(0);
    // 换了一条，卡片照新的重画：旧的码已经作废了。
    await expectCardFor(dialog, await linkText(page));

    await dialog.getByText('Invite by account ID').click();
    await expect(dialog.getByLabel('Account ID')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await expectAccessible(page, '.ar-dialog');
    await page.screenshot({ path: testInfo.outputPath(`invite-dialog-${String(width)}.png`) });

    await dialog.getByRole('button', { name: 'Turn off link' }).click();
    await expect(dialog).toContainText('The link is off. Nobody can use it now.');
    await expect(dialog.getByRole('button', { name: 'Make link and copy' })).toBeVisible();
    await expect(inviteCard(dialog)).toHaveCount(0);
    expect(failures).toEqual([]);
  });
}

test('邀请卡片：复制图片放进剪贴板，保存是一张以房间名起名的 PNG', async ({
  browserName,
  context,
  page,
}) => {
  // Playwright 只在 Chromium 上给得了剪贴板权限；别的浏览器上卡片画得对不对，上面那一条查。
  test.skip(browserName !== 'chromium', '剪贴板权限只有 Chromium 给得了');
  const failures = collectPageFailures(page);
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`${roomFixture}?private&invite-link`);
  await (await roomAction(page, 'Invite people')).click();
  const dialog = page.getByRole('dialog', { name: 'Invite people' });
  await expectCardFor(dialog, await linkText(page));
  // 看得到的明黄按钮只有一个：复制链接。卡片上的都是灰的。
  await expect(dialog.locator('.ar-button--primary:visible')).toHaveText(['Copy link']);

  await dialog.getByRole('button', { name: 'Copy image' }).click();
  await expect(dialog.getByRole('button', { name: 'Image copied' })).toBeVisible();
  expect(
    await page.evaluate(async () =>
      (await navigator.clipboard.read()).flatMap((item) => [...item.types]),
    ),
  ).toEqual(['image/png']);

  const download = page.waitForEvent('download');
  await dialog.getByRole('button', { name: 'Save image' }).click();
  expect((await download).suggestedFilename()).toBe('agent-room-Builders Exchange.png');
  expect(failures).toEqual([]);
});

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
  await expect(page.getByRole('button', { name: 'Room settings', exact: true })).toBeVisible();
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

test('桌面端邀请卡片“保存图片”：原生层存进“下载”，按钮下面说存成了什么、能在文件夹里指出来', async ({
  page,
}) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 1440, height: 900 });
  // 桌面端的窗口里页面自己的下载不出声（Windows 上悄悄存、Mac 上直接取消）：一次也不该走到。
  let pageDownloads = 0;
  page.on('download', () => {
    pageDownloads += 1;
  });
  await page.goto(`${roomFixture}?private&desktop`);
  await (await roomAction(page, 'Invite people')).click();
  const dialog = page.getByRole('dialog', { name: 'Invite people' });
  await dialog.getByRole('button', { name: 'Make link and copy' }).click();
  await expectCardFor(dialog, await linkText(page));
  const fixture = () =>
    page.evaluate(() => {
      const controls = (window as LobbyFixtureWindow).__agentRoomFixtureControls;
      return {
        saved: controls.savedFiles().map(({ contentBase64, fileName }) => {
          const bytes = Uint8Array.from(atob(contentBase64), (character) =>
            character.charCodeAt(0),
          );
          const view = new DataView(bytes.buffer);
          // PNG 的开头 8 个字节，和 IHDR 里的宽、高。
          return {
            fileName,
            signature: Array.from(bytes.subarray(0, 8)),
            size: [view.getUint32(16), view.getUint32(20)],
          };
        }),
        shown: controls.shownFiles(),
      };
    });

  await dialog.getByRole('button', { name: 'Save image' }).click();
  await expect(
    dialog.getByText('Saved to your Downloads folder as agent-room-Builders Exchange.png'),
  ).toBeVisible();
  // 交给原生层的就是画好的那张图：一张 1080×1350 的 PNG。
  expect(await fixture()).toEqual({
    saved: [
      {
        fileName: 'agent-room-Builders Exchange.png',
        signature: [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a],
        size: [1080, 1350],
      },
    ],
    shown: [],
  });
  await expectAccessible(page, '.share-card');

  await dialog.getByRole('button', { name: 'Show in folder' }).click();
  await expect
    .poll(async () => (await fixture()).shown)
    .toEqual(['agent-room-Builders Exchange.png']);

  // 再存一次是另一个文件，说的是新名字。
  await dialog.getByRole('button', { name: 'Save image' }).click();
  await expect(
    dialog.getByText('Saved to your Downloads folder as agent-room-Builders Exchange (1).png'),
  ).toBeVisible();
  expect(pageDownloads).toBe(0);
  expect(failures).toEqual([]);
});
