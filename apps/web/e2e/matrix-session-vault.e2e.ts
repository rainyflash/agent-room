import { expect, test } from '@playwright/test';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

const apiOrigin = 'https://api.agent-room.localhost:18443';

for (const boundary of ['human', 'matrix'] as const) {
  test(`桌面 ${boundary} 会话存储故障展示明确提示且重试可以恢复`, async ({ page, baseURL }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const errors: string[] = [];
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text());
    });
    await page.addInitScript((boundary) => {
      let loads = 0;
      let authorizations = 0;
      Object.defineProperty(window, 'isTauri', { value: true });
      Object.defineProperty(window, '__TAURI_EVENT_PLUGIN_INTERNALS__', {
        value: { unregisterListener: () => undefined },
      });
      Object.defineProperty(window, '__TAURI_INTERNALS__', {
        value: {
          transformCallback: () => 1,
          unregisterCallback: () => undefined,
          invoke: (command: string) => {
            if (command === 'plugin:event|listen') return Promise.resolve(1);
            if (command === 'plugin:event|unlisten') return Promise.resolve();
            if (command === 'desktop_begin_matrix_authentication') {
              document.documentElement.dataset.matrixAuthorizations = String(++authorizations);
              // Leave the external authorization open so the UI can expose its pending state.
              return new Promise<never>(() => undefined);
            }
            if (command === 'desktop_restore_human_session') {
              if (boundary === 'matrix' || loads++ > 0) return Promise.resolve(true);
              return Promise.reject(
                new Error(
                  JSON.stringify({
                    code: 'desktop.human_session.vault_unavailable',
                    retryable: true,
                  }),
                ),
              );
            }
            if (command === 'desktop_load_matrix_session') {
              ++loads;
              return boundary === 'matrix' && loads === 1
                ? Promise.reject(
                    new Error(
                      JSON.stringify({
                        code: 'desktop.matrix_session.vault_unavailable',
                        retryable: true,
                      }),
                    ),
                  )
                : Promise.resolve(null);
            }
            return Promise.reject(
              new Error(
                JSON.stringify({ code: 'desktop.test.runtime_unavailable', retryable: true }),
              ),
            );
          },
        },
      });
    }, boundary);
    await page.route(`${apiOrigin}/**`, async (route) => {
      const path = new URL(route.request().url()).pathname;
      const headers = {
        'access-control-allow-origin': new URL(baseURL ?? '').origin,
        'access-control-allow-credentials': 'true',
        'content-type': 'application/json',
      };
      const body =
        path === '/auth/session'
          ? {
              authenticatedAtUnixMs: Date.now(),
              displayName: 'Session Test',
              expiresAtUnixMs: Date.now() + 60_000,
              locale: 'en',
              matrixUserId: '@tester:matrix.test',
              principalId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
              recentlyAuthenticated: true,
            }
          : {
              checkedAtUnixMs: Date.now(),
              correlationId: '018c251e-7b5a-7c7f-8a28-2de53f56a9a3',
              dependencies: [
                { latencyMs: 1, name: 'database', status: 'available' },
                { latencyMs: 1, name: 'matrix', status: 'available' },
              ],
              service: 'agent-room-control-plane',
              status: 'ready',
              version: '0.1.0',
            };
      await route.fulfill({ status: 200, headers, body: JSON.stringify(body) });
    });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto('/connect');
    await expect(page).toHaveTitle('Agent Room');
    await expect(page.getByRole('alert')).toContainText(/system credential store|系统凭据库/u);
    const diagnostic = page.getByText(`desktop.${boundary}_session.vault_unavailable`, {
      exact: true,
    });
    await expect(diagnostic).toBeHidden();
    await page.locator('.connection-service-details summary').click();
    await expect(diagnostic).toBeVisible();
    await page.locator('.connection-service-details summary').click();
    await expect(page.locator('vite-error-overlay')).toHaveCount(0);
    await page.screenshot({
      path: join(tmpdir(), `agent-room-${boundary}-vault-desktop.png`),
      fullPage: true,
    });
    await page.setViewportSize({ width: 390, height: 844 });
    await expect
      .poll(() => page.evaluate(() => document.documentElement.scrollWidth))
      .toBeLessThanOrEqual(390);
    const retry = page.getByRole('button', { name: /^(Retry now|Reconnect|立即重试|重新连接)$/u });
    await expect(retry).toBeEnabled();
    await page.screenshot({
      path: join(tmpdir(), `agent-room-${boundary}-vault-mobile.png`),
      fullPage: true,
    });
    await retry.click();
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(
      /Connecting your conversations|正在连接你的对话/u,
    );
    await expect(page.locator('html')).toHaveAttribute('data-matrix-authorizations', '1');
    await expect(page.locator('.failure-panel')).toHaveCount(0);
    expect(errors).toEqual([]);
  });
}
