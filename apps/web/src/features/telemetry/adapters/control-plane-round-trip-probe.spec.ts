import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneRoundTripProbe } from './control-plane-round-trip-probe';

function clock(...readings: number[]): () => number {
  const queue = [...readings];
  return () => queue.shift() ?? Number.NaN;
}

describe('ControlPlaneRoundTripProbe', () => {
  it('问一次不要登录的存活接口，量出来回的毫秒数', async () => {
    const fetchImplementation = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response(null, { status: 200 }));
    const probe = new ControlPlaneRoundTripProbe({
      baseUrl: 'https://app.agent-room.test/_agent-room/api',
      fetch: fetchImplementation,
      now: clock(1_000, 1_180),
    });

    await expect(probe.measure()).resolves.toBe(180);

    const [url, request] = fetchImplementation.mock.calls[0] ?? [];
    expect(url).toEqual(new URL('https://app.agent-room.test/_agent-room/api/health/live'));
    expect(request).toMatchObject({ cache: 'no-store', credentials: 'omit', method: 'GET' });
  });

  it('服务器没答好或者网络断了就不算', async () => {
    const failing = new ControlPlaneRoundTripProbe({
      baseUrl: 'https://api.agent-room.test',
      fetch: vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 503 })),
      now: clock(0, 50),
    });
    await expect(failing.measure()).resolves.toBeNull();

    const offline = new ControlPlaneRoundTripProbe({
      baseUrl: 'https://api.agent-room.test',
      fetch: vi.fn<typeof fetch>().mockRejectedValue(new TypeError('offline')),
      now: clock(0, 50),
    });
    await expect(offline.measure()).resolves.toBeNull();
  });
});
