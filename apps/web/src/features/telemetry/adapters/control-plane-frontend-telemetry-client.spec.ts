import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneFrontendTelemetryClient } from './control-plane-frontend-telemetry-client';

describe('ControlPlaneFrontendTelemetryClient', () => {
  it('只发送固定指标、表面和数值，不携带身份或页面地址', async () => {
    const fetchImplementation = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response(null, { status: 204 }));
    const client = new ControlPlaneFrontendTelemetryClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: fetchImplementation,
    });

    await client.record({
      metric: 'largest_contentful_paint',
      surface: 'web',
      value: 1_250,
    });

    expect(fetchImplementation).toHaveBeenCalledTimes(1);
    const [url, request] = fetchImplementation.mock.calls[0] ?? [];
    expect(url).toEqual(new URL('https://api.agent-room.test/telemetry/frontend'));
    expect(request).toMatchObject({
      body: JSON.stringify({
        metric: 'largest_contentful_paint',
        surface: 'web',
        value: 1_250,
      }),
      credentials: 'include',
      keepalive: true,
      method: 'POST',
    });
    const body = request?.body;
    expect(typeof body).toBe('string');
    if (typeof body === 'string') {
      expect(body).not.toMatch(/principal|agent|room|message|token|https?:/iu);
    }
  });

  it('网络失败时保持旁路，不向产品流程抛错', async () => {
    const client = new ControlPlaneFrontendTelemetryClient({
      baseUrl: 'https://api.agent-room.test',
      fetch: vi.fn<typeof fetch>().mockRejectedValue(new TypeError('offline')),
    });

    await expect(
      client.record({ metric: 'message_open', surface: 'desktop', value: 80 }),
    ).resolves.toBeUndefined();
  });
});
