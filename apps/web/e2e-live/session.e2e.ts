import { expect, test } from '@playwright/test';

import { collectUnhandledFailures, connectLiveSession } from './support/live-session';
const username = process.env.AGENT_ROOM_E2E_USERNAME;
const password = process.env.AGENT_ROOM_E2E_PASSWORD;

test('OIDC 与 Matrix SSO 建立同一主体并能刷新恢复', async ({ page }) => {
  test.skip(username === undefined || password === undefined, '缺少隔离验收账户。');
  const failures = collectUnhandledFailures(page);

  const firstIdentity = await connectLiveSession(page, username ?? '', password ?? '');

  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    /Connection established|连接已建立/u,
    { timeout: 40_000 },
  );
  await expect(page.locator('.identity-summary dd').first()).toHaveText(firstIdentity);
  expect(failures).toEqual([]);
});
