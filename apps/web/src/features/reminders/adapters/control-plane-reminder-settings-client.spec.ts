import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneReminderSettingsClient } from './control-plane-reminder-settings-client';

const BASE_URL = 'https://app.agent-room.test/_agent-room/api';

describe('ControlPlaneReminderSettingsClient', () => {
  it('读设置带上登录、不缓存', async () => {
    const fetch = vi.fn(() =>
      Promise.resolve(Response.json({ email: { enabled: true, available: true } })),
    );
    const client = new ControlPlaneReminderSettingsClient({ baseUrl: BASE_URL, fetch });

    expect(await client.read()).toEqual({
      ok: true,
      value: { email: { enabled: true, available: true } },
    });
    const [url, init] = fetch.mock.calls[0] as unknown as [URL, RequestInit];
    expect(url.toString()).toBe(`${BASE_URL}/account/reminders`);
    expect(init.method).toBe('GET');
    expect(init.credentials).toBe('include');
    expect(init.cache).toBe('no-store');
    expect(init.body).toBeUndefined();
  });

  it('改开关用 PUT，正文只有邮件那一项', async () => {
    const fetch = vi.fn(() =>
      Promise.resolve(Response.json({ email: { enabled: false, available: true } })),
    );
    const client = new ControlPlaneReminderSettingsClient({ baseUrl: BASE_URL, fetch });

    expect(await client.setEmail(false)).toEqual({
      ok: true,
      value: { email: { enabled: false, available: true } },
    });
    const [, init] = fetch.mock.calls[0] as unknown as [URL, RequestInit];
    expect(init.method).toBe('PUT');
    expect(init.body).toBe('{"email":{"enabled":false}}');
    expect(new Headers(init.headers).get('Content-Type')).toBe('application/json');
  });

  it('回答多了字段就不认：服务器和网页对不上时别当成功', async () => {
    const fetch = vi.fn(() =>
      Promise.resolve(Response.json({ email: { enabled: true, available: true, extra: 1 } })),
    );
    const client = new ControlPlaneReminderSettingsClient({ baseUrl: BASE_URL, fetch });

    expect(await client.read()).toEqual({
      ok: false,
      error: { code: 'reminders.invalid_response', retryable: false },
    });
  });

  it('服务器的错误照原样带回来，连不上算能重试', async () => {
    const failing = vi.fn(() =>
      Promise.resolve(
        Response.json(
          { code: 'reminders.unavailable', correlationId: 'cid-1', retryable: true },
          { status: 503 },
        ),
      ),
    );
    expect(
      await new ControlPlaneReminderSettingsClient({ baseUrl: BASE_URL, fetch: failing }).setEmail(
        true,
      ),
    ).toEqual({
      ok: false,
      error: { code: 'reminders.unavailable', correlationId: 'cid-1', retryable: true },
    });

    const plain = vi.fn(() => Promise.resolve(new Response('nope', { status: 404 })));
    expect(
      await new ControlPlaneReminderSettingsClient({ baseUrl: BASE_URL, fetch: plain }).read(),
    ).toEqual({ ok: false, error: { code: 'reminders.http_404', retryable: false } });

    const offline = vi.fn(() => Promise.reject(new TypeError('Failed to fetch')));
    expect(
      await new ControlPlaneReminderSettingsClient({ baseUrl: BASE_URL, fetch: offline }).read(),
    ).toEqual({ ok: false, error: { code: 'reminders.unreachable', retryable: true } });
  });
});
