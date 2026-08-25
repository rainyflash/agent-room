import { expect, test, type Page } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.route('https://api.agent-room.localhost:18443/**', async (route) => {
    const url = new URL(route.request().url());
    const headers = {
      'access-control-allow-credentials': 'true',
      'access-control-allow-origin': 'http://127.0.0.1:14173',
      'content-type': 'application/json',
    };
    if (url.pathname === '/auth/session') {
      await route.fulfill({ body: '{}', headers, status: 401 });
      return;
    }
    if (url.pathname === '/health/ready') {
      await route.fulfill({
        body: JSON.stringify({
          checkedAtUnixMs: 1_700_000_000_000,
          correlationId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
          dependencies: [],
          service: 'agent-room-control-plane',
          status: 'ready',
          version: '0.1.0',
        }),
        headers,
        status: 200,
      });
      return;
    }
    await route.fulfill({ body: '{}', headers, status: 404 });
  });
});

test('账户语言与设备临时覆盖即时生效', async ({ page }) => {
  await page.goto('/connect');
  const language = page.getByRole('combobox', { name: /Language|语言/u });

  await language.selectOption('account:en');
  await expect(page.locator('html')).toHaveAttribute('lang', 'en');
  await expect(page.getByRole('heading', { level: 1 })).toContainText(/sign-in|offline/u);

  await language.selectOption('device:zh-CN');
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN');
  await expect(page.getByRole('heading', { level: 1 })).toContainText(/登录|离线/u);
});

test('中英长字符串膨胀和缺失中文分支不会破坏布局', async ({ page }) => {
  await page.setViewportSize({ height: 900, width: 1_280 });
  await page.goto('/connect');
  await page.getByRole('combobox', { name: /Language|语言/u }).selectOption('account:en');

  await installExpandedEnglishCatalog(page);
  await expect(page.getByRole('heading', { level: 1 })).toContainText(/sign-in|offline/u);
  await expectNoHorizontalOverflow(page);

  await page.setViewportSize({ height: 844, width: 390 });
  await expectNoHorizontalOverflow(page);
  await expect(page.getByRole('button', { name: /Sign in|Retry/u })).toBeVisible();

  const fallback = await removeChineseMessageAndUseFallback(page, 'app.environment');
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN');
  expect(fallback).toContain('FEDERATED OPERATIONS LOBBY');
  await expect(page.locator('body')).not.toContainText('app.environment');
});

async function installExpandedEnglishCatalog(page: Page): Promise<void> {
  await page.evaluate(async () => {
    const modulePath = '/src/shared/i18n/i18n.ts';
    const loaded: unknown = await import(modulePath);
    const module = loaded as {
      readonly i18n: {
        addResourceBundle(
          language: string,
          namespace: string,
          resources: Record<string, unknown>,
          deep: boolean,
          overwrite: boolean,
        ): void;
        changeLanguage(language: string): Promise<unknown>;
        getResourceBundle(language: string, namespace: string): unknown;
      };
    };
    const catalog = module.i18n.getResourceBundle('en', 'translation') as Record<string, unknown>;
    const expanded = Object.fromEntries(
      Object.entries(catalog).map(([key, value]) => [
        key,
        typeof value === 'string' && value.length > 8 ? `${value} — ${value}` : value,
      ]),
    );
    module.i18n.addResourceBundle('en', 'translation', expanded, true, true);
    await module.i18n.changeLanguage('zh-CN');
    await module.i18n.changeLanguage('en');
  });
}

async function removeChineseMessageAndUseFallback(page: Page, key: string): Promise<string> {
  return page.evaluate(async (messageKey) => {
    const modulePath = '/src/shared/i18n/i18n.ts';
    const loaded: unknown = await import(modulePath);
    const module = loaded as {
      readonly i18n: {
        getResourceBundle(language: string, namespace: string): unknown;
        t(key: string): string;
      };
      setDeviceLanguageOverride(override: 'zh-CN', account: 'en'): Promise<void>;
    };
    const catalog = module.i18n.getResourceBundle('zh-CN', 'translation') as Record<
      string,
      unknown
    >;
    Reflect.deleteProperty(catalog, messageKey);
    await module.setDeviceLanguageOverride('zh-CN', 'en');
    return module.i18n.t(messageKey);
  }, key);
}

async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  const dimensions = await page.evaluate(() => ({
    clientWidth: document.documentElement.clientWidth,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(dimensions.scrollWidth).toBeLessThanOrEqual(dimensions.clientWidth);
}
