import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import { applicationVersion } from '../build/runtime-manifest';
import { readFile } from 'node:fs/promises';
import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

type FeatureWindow = Window &
  typeof globalThis & {
    readonly __applicationFeatureEvidence: {
      readonly creates: readonly string[];
      readonly uploads: readonly { readonly submissionId: string; readonly digest: string }[];
      readonly publishes: number;
      readonly binds: number;
    };
  };
const fixture = '/e2e/fixtures/lobby-scene.html?features=1';
const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6c3sAAAAASUVORK5CYII=',
  'base64',
);

for (const width of [1440, 390]) {
  test(`新建房间、换个房间与版本入口 ${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 900 });
    await page.goto(`${fixture}&failJoin=1`);
    await expect(page.getByRole('heading', { name: 'Your private rooms' })).toBeVisible();
    await page.getByRole('button', { name: 'New room', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: 'New room' });
    await dialog.getByRole('textbox', { name: 'Name', exact: true }).fill('Release workshop');
    await expect(dialog).toContainText('Private rooms are invite only');
    const bounds = await dialog.boundingBox();
    if (bounds === null) throw new Error('New room dialog is not visible');
    expect(Math.abs(bounds.x + bounds.width / 2 - width / 2)).toBeLessThan(2);
    if (width >= 768) expect(Math.abs(bounds.y + bounds.height / 2 - 450)).toBeLessThan(2);
    // 手机上是贴底的面板。
    else expect(Math.abs(bounds.y + bounds.height - 900)).toBeLessThan(2);
    await page.screenshot({
      path: testInfo.outputPath(`create-${String(width)}.png`),
      fullPage: true,
    });
    await dialog.getByRole('button', { name: 'Create and enter' }).click();
    await expect(dialog.getByRole('alert')).toContainText('Could not finish creating the room');
    await dialog.getByRole('button', { name: 'Try again', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(
      page.getByRole('heading', { name: 'Release workshop', exact: true }),
    ).toBeVisible();
    const evidence = await page.evaluate(
      () => (window as FeatureWindow).__applicationFeatureEvidence,
    );
    expect(evidence.creates).toHaveLength(2);
    expect(new Set(evidence.creates).size).toBe(1);
    const input = page.getByRole('textbox', { name: 'Message', exact: true });
    await input.fill('Keep my draft here');
    await page.getByRole('button', { name: 'Switch room' }).click();
    await page.getByRole('searchbox', { name: 'Search rooms' }).fill('Engineering');
    await page
      .getByRole('dialog')
      .getByRole('button', { name: /Engineering room/ })
      .click();
    await expect(
      page.getByRole('heading', { name: 'Engineering room', exact: true }),
    ).toBeVisible();
    await expect(input).toHaveValue('');
    await page.getByRole('button', { name: 'Switch room' }).click();
    await page
      .getByRole('dialog')
      .getByRole('button', { name: /Release workshop/ })
      .click();
    await expect(input).toHaveValue('Keep my draft here');
    // 关于与更新并进了设置：顶栏“设置” → “关于”。
    await page.getByRole('link', { name: 'Settings', exact: true }).click();
    await page
      .getByRole('navigation', { name: 'Settings sections' })
      .getByRole('link', { name: 'About' })
      .click();
    await expect(page.getByRole('heading', { name: 'Application updates' })).toBeVisible();
    await expect(page.getByRole('main')).toContainText(applicationVersion);
    await page.getByRole('button', { name: 'Check', exact: true }).click();
    await expect(page.getByRole('status')).toContainText('up to date');
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
    await page.screenshot({
      path: testInfo.outputPath(`about-${String(width)}.png`),
      fullPage: true,
    });
  });

  test(`附件草稿、图片和加密文件重试 ${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 950 });
    await page.goto(`${fixture}&failUpload=1`);
    await page.getByRole('link', { name: /Design studio/ }).click();
    const input = page.getByRole('textbox', { name: 'Message', exact: true });
    await expect(input).toBeEnabled();
    await page
      .getByLabel('Attach a file', { exact: true })
      .setInputFiles({ name: 'diagram.png', mimeType: 'image/png', buffer: png });
    await expect(page.locator('.conversation-attachment-picker__file')).toContainText(
      'diagram.png',
    );
    await page.getByRole('button', { name: 'Switch room' }).click();
    await page
      .getByRole('dialog')
      .getByRole('button', { name: /Engineering room/ })
      .click();
    await expect(page.locator('.conversation-attachment-picker__file')).toHaveCount(0);
    await page.getByRole('button', { name: 'Switch room' }).click();
    await page
      .getByRole('dialog')
      .getByRole('button', { name: /Design studio/ })
      .click();
    await expect(page.locator('.conversation-attachment-picker__file img')).toBeVisible();
    await input.fill('Please review this image');
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(page.getByRole('alert')).toContainText('could not');
    await page.getByRole('button', { name: 'Retry this message', exact: true }).click();
    await expect(page.getByRole('log')).toContainText('diagram.png');
    await page.getByRole('button', { name: 'View image' }).click();
    await expect(page.getByRole('log').getByRole('img', { name: 'diagram.png' })).toBeVisible();
    const bytes = Buffer.alloc(1024 * 1024, 0xa5);
    await page
      .getByLabel('Attach a file', { exact: true })
      .setInputFiles({ name: 'sample.bin', mimeType: 'application/octet-stream', buffer: bytes });
    await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
    await page.getByRole('button', { name: 'Send', exact: true }).click();
    await expect(page.getByRole('log')).toContainText('sample.bin');
    await page.getByRole('button', { name: 'Load file' }).click();
    const downloadEvent = page.waitForEvent('download');
    await page.getByRole('link', { name: 'Download file', exact: true }).last().click();
    const download = await downloadEvent;
    const path = await download.path();
    expect(await readFile(path)).toEqual(bytes);
    const evidence = await page.evaluate(
      () => (window as FeatureWindow).__applicationFeatureEvidence,
    );
    expect(evidence.uploads[0]).toEqual(evidence.uploads[1]);
    expect(evidence.publishes).toBe(2);
    expect(evidence.binds).toBe(2);
    await expectNoHorizontalOverflow(page);
    expect(failures).toEqual([]);
    await page.screenshot({
      path: testInfo.outputPath(`attachments-${String(width)}.png`),
      fullPage: true,
    });
  });
}

test('别人的邀请：在“房间”页拒绝，在“换个房间”里加入', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.goto(fixture);
  const invitations = page.getByRole('list', { name: 'Invitations' });
  await expect(invitations).toContainText('Research lab');
  // Research lab 是网络 Agent 建的：接受之前就说服务器能读到；人建的 Budget review 不说。
  const serverReadable = 'A network agent made this room. The server can read what is said here.';
  const invitation = (name: string) => invitations.getByRole('listitem').filter({ hasText: name });
  await expect(invitation('Research lab')).toContainText(serverReadable);
  await expect(invitation('Budget review')).not.toContainText(serverReadable);
  await invitations
    .getByRole('listitem')
    .filter({ hasText: 'Budget review' })
    .getByRole('button', { name: 'Decline' })
    .click();
  await expect(invitations).not.toContainText('Budget review');
  await page.getByRole('link', { name: /Design studio/ }).click();
  // 还有一个邀请没答复：“换个房间”上带着个数。
  await page.getByRole('button', { name: /Switch room.*1 invitation/ }).click();
  const switcher = page.getByRole('dialog', { name: 'Switch room' });
  await expect(switcher.getByRole('region', { name: 'Public lobbies' })).toContainText(
    'Global lobby',
  );
  await expect(switcher.getByRole('region', { name: 'Invitations' })).toContainText(serverReadable);
  const scan = await new AxeBuilder({ page })
    .include('.ar-dialog')
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  expect(scan.violations.map(({ id }) => id)).toEqual([]);
  await switcher
    .getByRole('region', { name: 'Invitations' })
    .getByRole('button', { name: 'Join' })
    .click();
  await expect(page.getByRole('heading', { name: 'Research lab', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Switch room', exact: true })).toBeVisible();
  expect(failures).toEqual([]);
});

for (const width of [1440, 390]) {
  test(`已归档的房间单独收起来，点开能进去看 ${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 900 });
    await page.goto(fixture);
    const mine = page.getByRole('region', { name: 'Your private rooms' });
    const archived = mine.locator('details', { hasText: 'Archived rooms (1)' });
    await expect(archived).toBeVisible();
    // 默认收起：卡片看不见；上面已加入的房间里也没有它。
    const card = mine.getByRole('link', { name: /Spring launch/ });
    await expect(card).toBeHidden();
    await expect(mine.getByRole('link', { name: /Design studio/ })).toBeVisible();
    await archived.getByText('Archived rooms (1)').click();
    await expect(archived).toContainText('Nobody can speak in these rooms any more.');
    await expect(card).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath(`archived-rooms-${String(width)}.png`),
      fullPage: true,
    });
    await card.click();
    await expect(page.getByRole('heading', { name: 'Spring launch', exact: true })).toBeVisible();
    expect(failures).toEqual([]);
  });
}

for (const width of [1440, 390]) {
  test(`你建的和你加入的分开，别人的房间写着房主 ${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 900 });
    await page.goto(fixture);
    const mine = page.getByRole('region', { name: 'Your private rooms' });
    const created = mine.getByRole('region', { name: 'Rooms you created' });
    const joined = mine.getByRole('region', { name: 'Rooms you joined' });
    // Design studio 是你的 Agent 替你建的，房主是你：归“你建的”，不写房主。
    await expect(created.getByRole('link', { name: /Design studio/ })).toBeVisible();
    await expect(created.getByRole('link', { name: /Engineering room/ })).toBeVisible();
    await expect(created).not.toContainText('Owner:');
    await expect(joined.getByRole('link', { name: /Partner workshop/ })).toContainText(
      'Owner: Mia',
    );
    // 网络 Agent 当房主：写它的名字，不写替它记账的合成主体。
    const fieldNotes = joined.getByRole('link', { name: /Field notes/ });
    await expect(fieldNotes).toContainText('Owner: Atlas (network agent)');
    await expect(joined).not.toContainText('network-agent-principal');
    await expect(joined.getByRole('link')).toHaveCount(2);
    // 邀请照旧在最上面，已归档的照旧收在最下面。
    const invitations = await mine.getByRole('list', { name: 'Invitations' }).boundingBox();
    const createdBox = await created.boundingBox();
    const joinedBox = await joined.boundingBox();
    const archivedBox = await mine
      .locator('details', { hasText: 'Archived rooms (1)' })
      .boundingBox();
    if (invitations === null || createdBox === null || joinedBox === null || archivedBox === null)
      throw new Error('A room group is not visible');
    expect(invitations.y).toBeLessThan(createdBox.y);
    expect(createdBox.y).toBeLessThan(joinedBox.y);
    expect(joinedBox.y).toBeLessThan(archivedBox.y);
    await expectNoHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath(`rooms-by-owner-${String(width)}.png`),
      fullPage: true,
    });
    await fieldNotes.click();
    await expect(page.getByRole('heading', { name: 'Field notes', exact: true })).toBeVisible();
    expect(failures).toEqual([]);
  });
}

test('中文：“你建的房间”“你加入的房间”和房主', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ width: 390, height: 900 });
  await page.addInitScript(() => {
    localStorage.setItem('agent-room.language', 'zh-CN');
  });
  await page.goto(fixture);
  const mine = page.getByRole('region', { name: '你的私人房间' });
  await expect(mine.getByRole('region', { name: '你建的房间' })).toContainText('Design studio');
  const joined = mine.getByRole('region', { name: '你加入的房间' });
  await expect(joined.getByRole('link', { name: /Partner workshop/ })).toContainText('房主：Mia');
  await expect(joined.getByRole('link', { name: /Field notes/ })).toContainText(
    '房主：Atlas（网络 Agent）',
  );
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

for (const width of [1440, 390]) {
  test(`房间卡片写未读条数和有没有人点名你 ${String(width)}`, async ({ page }, testInfo) => {
    const failures = collectPageFailures(page);
    await page.setViewportSize({ width, height: 900 });
    await page.goto(`${fixture}&unread`);
    const mine = page.getByRole('region', { name: 'Your private rooms' });
    // Design studio 读到一半，后面三条里有一条点名你；Engineering room 积了 120 条。
    await expect(mine.getByRole('link', { name: /Design studio/ })).toContainText(
      'Someone mentioned you, 3 unread messages',
    );
    await expect(mine.getByRole('link', { name: /Engineering room/ })).toContainText(
      'At least 99 unread messages',
    );
    await expect(mine.getByRole('link', { name: /Engineering room/ })).toContainText('99+');
    // 公开大厅只标 @，不写条数。
    const lobby = page.locator('.room-card', { hasText: 'Global lobby' });
    await expect(lobby).toContainText('Someone mentioned you');
    await expect(lobby.locator('.room-unread-mark__count')).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
    await page.screenshot({
      path: testInfo.outputPath(`room-unread-${String(width)}.png`),
      fullPage: true,
    });
    expect(failures).toEqual([]);
  });
}

test('刷新恢复附件、清除后不残留本地文件', async ({ page }) => {
  await page.goto(fixture);
  await page.getByRole('link', { name: /Design studio/ }).click();
  await page
    .getByLabel('Attach a file', { exact: true })
    .setInputFiles({ name: 'restore.png', mimeType: 'image/png', buffer: png });
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
  await page.reload();
  await page.getByRole('link', { name: /Design studio/ }).click();
  await expect(page.locator('.conversation-attachment-picker__file')).toContainText('restore.png');
  await expect(page.locator('.conversation-attachment-picker__file img')).toBeVisible();
  await page.getByRole('button', { name: 'Remove attachment' }).click();
  await expect(page.locator('.conversation-attachment-picker__file')).toHaveCount(0);
  await page.reload();
  await page.getByRole('link', { name: /Design studio/ }).click();
  await expect(page.locator('.conversation-attachment-picker__file')).toHaveCount(0);
});

test('粘贴与拖入附件，关闭窗口后复用原密文和提交编号续发', async ({ page, context }) => {
  await page.goto(`${fixture}&failUpload=1`);
  await page.getByRole('link', { name: /Design studio/ }).click();
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await expect(input).toBeEnabled();
  await input.evaluate(
    (element, bytes) => {
      const clipboard = new DataTransfer();
      clipboard.items.add(new File([Uint8Array.from(bytes)], 'pasted.png', { type: 'image/png' }));
      element.dispatchEvent(
        new ClipboardEvent('paste', { bubbles: true, clipboardData: clipboard }),
      );
    },
    [...png],
  );
  await expect(page.locator('.conversation-attachment-picker__file')).toContainText('pasted.png');
  await page.getByRole('button', { name: 'Remove attachment' }).click();
  await page.locator('.conversation-panel__composer').evaluate(
    (element, bytes) => {
      const dropped = new DataTransfer();
      dropped.items.add(new File([Uint8Array.from(bytes)], 'dropped.png', { type: 'image/png' }));
      element.dispatchEvent(
        new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: dropped }),
      );
    },
    [...png],
  );
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('could not');
  const first = await page.evaluate(
    () => (window as FeatureWindow).__applicationFeatureEvidence.uploads[0],
  );
  await page.close();
  const reopened = await context.newPage();
  await reopened.goto(fixture);
  await reopened.getByRole('link', { name: /Design studio/ }).click();
  await expect(reopened.getByRole('log')).toContainText('dropped.png');
  const restored = await reopened.evaluate(
    () => (window as FeatureWindow).__applicationFeatureEvidence,
  );
  expect(restored.uploads).toEqual([first]);
  expect(restored.publishes).toBe(1);
  await reopened.getByRole('button', { name: 'View image' }).click();
  await expect(reopened.getByRole('log').getByRole('img', { name: 'dropped.png' })).toBeVisible();
});
