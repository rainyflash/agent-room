import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';

import { collectPageFailures, expectNoHorizontalOverflow } from './support/page-assertions';

const apiOrigin = 'https://api.agent-room.localhost:18443';
const configuredDownloadUrl = normalizedDownloadUrl(
  process.env.VITE_AGENT_ROOM_WINDOWS_DOWNLOAD_URL,
);
const downloadState = configuredDownloadUrl === null ? 'pending' : 'published';
const registrationOpen = process.env.VITE_AGENT_ROOM_IDENTITY_REGISTRATION_MODE === 'open-email';
const wcagTags = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] as const;

test('公开首页呈现真实 Alpha 入口并进入房间入口', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 960, width: 1_440 });
  await page.goto('/');

  await expect(page).toHaveTitle('Agent Room');
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /A room for you and your agents|你和 Agent，相聚一个房间/u,
  );
  await expect(page.getByRole('button', { name: /Log in|登录/u })).toBeVisible();
  const registration = registrationOpen
    ? page.getByRole('button', { name: /Create account|注册账户/u })
    : page.getByRole('button', { name: /Registration coming soon|注册即将开放/u });
  await expect(registration).toBeVisible();
  if (!registrationOpen) {
    await expect(registration).toBeDisabled();
  }

  if (configuredDownloadUrl === null) {
    await expect(
      page.getByRole('button', { name: /Windows download unavailable|Windows 下载暂不可用/u }),
    ).toBeDisabled();
    await expect(
      page.getByRole('link', { name: /Download for Windows|下载 Windows 应用/u }),
    ).toHaveCount(0);
  } else {
    await expect(
      page.getByRole('link', { name: /Download for Windows|下载 Windows 应用/u }),
    ).toHaveAttribute('href', configuredDownloadUrl);
  }

  await expectNoHorizontalOverflow(page);
  const accessibility = await new AxeBuilder({ page }).withTags([...wcagTags]).analyze();
  expect(accessibility.violations).toEqual([]);
  expect(failures).toEqual([]);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath(`landing-${downloadState}-desktop.png`),
  });

  await page.getByRole('link', { name: /Enter the room|进入房间/u }).click();
  await expect(page).toHaveURL(/\/connect$/u);
  await expect(page.getByRole('list', { name: /Connection progress|连接进度/u })).toBeVisible();
  await expect(page.locator('.connection-step')).toHaveCount(5);
});

test('注册入口只发送 Agent Room 注册意图', async ({ page }) => {
  await page.route(`${apiOrigin}/auth/oidc/start?**`, async (route) => {
    await route.fulfill({
      body: '<!doctype html><title>Registration boundary</title>',
      contentType: 'text/html',
      status: 200,
    });
  });
  await page.goto('/');

  if (!registrationOpen) {
    await expect(
      page.getByRole('button', { name: /Registration coming soon|注册即将开放/u }),
    ).toBeDisabled();
    return;
  }

  await page.getByRole('button', { name: /Create account|注册账户/u }).click();

  const target = new URL(page.url());
  expect(target.origin).toBe(apiOrigin);
  expect(target.pathname).toBe('/auth/oidc/start');
  expect(target.searchParams.get('returnTo')).toBe('/connect');
  expect(target.searchParams.get('intent')).toBe('register');
});

test('公开首页在 390px 视口保持主入口可用', async ({ page }, testInfo) => {
  const failures = collectPageFailures(page);
  await page.setViewportSize({ height: 844, width: 390 });
  await page.goto('/');

  const preview = page.getByRole('link', { name: /Enter the room|进入房间/u });
  await expect(preview).toBeVisible();
  expect((await preview.boundingBox())?.height ?? 0).toBeGreaterThanOrEqual(44);
  await expectNoHorizontalOverflow(page);
  expect(failures).toEqual([]);
  await page.screenshot({
    animations: 'disabled',
    fullPage: true,
    path: testInfo.outputPath(`landing-${downloadState}-mobile.png`),
  });
});

function normalizedDownloadUrl(value: string | undefined): string | null {
  const normalized = value?.trim();
  return normalized === undefined || normalized.length === 0 ? null : normalized;
}
