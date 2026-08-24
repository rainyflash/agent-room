import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const fixturePath = '/e2e/fixtures/lobby-scene.html';

test('桌面发送入口不被控制坞遮挡，并可完成签名发送流程', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 900, width: 1_440 });
  await page.goto(fixturePath);

  const launcher = page.getByRole('button', { name: 'Open the message composer' });
  const controls = page.locator('.signal-dock');
  await expect(launcher).toBeVisible();
  await expect(controls).toBeVisible();
  const [launcherBox, controlsBox] = await Promise.all([
    launcher.boundingBox(),
    controls.boundingBox(),
  ]);
  expect(launcherBox).not.toBeNull();
  expect(controlsBox).not.toBeNull();
  expect((launcherBox?.y ?? 0) + (launcherBox?.height ?? 0)).toBeLessThan(controlsBox?.y ?? 0);

  await launcher.click();
  const composer = page.getByRole('complementary', { name: 'Compose room signal' });
  await expect(composer).toBeVisible();
  await expect(composer).toContainText('Build Agent');
  await expect(composer).toContainText('Builders Exchange');

  await page.getByLabel('Preview title').fill('Protocol review');
  await page.getByLabel('Preview summary').fill('Please inspect the protocol change.');
  await page.getByLabel('Full content').fill('Review https://example.com and keep <script> inert.');
  await expect(composer).toContainText('External links detected');
  await expect(composer).toContainText('HTML markup will remain inert');

  await page.getByRole('button', { name: 'Sign and send' }).click();
  await expect(composer).toContainText('Message accepted');
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});

test('窄屏发送器占满可用宽度且保留底部安全区', async ({ page }) => {
  const failures = collectPageFailures(page);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto(fixturePath);

  await page.getByRole('button', { name: 'Open the message composer' }).click();
  const composer = page.getByRole('complementary', { name: 'Compose room signal' });
  await expect(composer).toBeVisible();
  await expect(page.locator('.message-dock')).toBeHidden();
  const composerBox = await composer.boundingBox();
  expect(composerBox).not.toBeNull();
  expect(composerBox?.x).toBe(0);
  expect(composerBox?.width).toBe(390);
  expect((composerBox?.y ?? 0) + (composerBox?.height ?? 0)).toBeLessThanOrEqual(788);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
});
