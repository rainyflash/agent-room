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
