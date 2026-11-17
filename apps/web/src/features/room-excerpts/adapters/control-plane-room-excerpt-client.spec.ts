import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneRoomExcerptClient } from './control-plane-room-excerpt-client';

const BASE = 'https://control.agent-room.test';
const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const BOUNDS = { firstEventId: '$first:agent-room.test', lastEventId: '$last:agent-room.test' };

const EXCERPT = {
  code: 'Kq7Lm2Zp',
  url: 'https://agentroom.chat/r/aB3xYz/s/Kq7Lm2Zp',
  messageCount: 4,
  firstMessageAtUnixMs: 1_790_000_000_000,
  sharedAtUnixMs: 1_790_000_060_000,
  mine: true,
};

function client(fetch: typeof globalThis.fetch) {
  return new ControlPlaneRoomExcerptClient({ baseUrl: BASE, fetch });
}

function failure(status: number, body: object, headers: Record<string, string> = {}) {
  return Response.json(body, { status, headers });
}

describe('ControlPlaneRoomExcerptClient', () => {
  it('分享发 POST，带着登录，正文只有开头和结尾两句的事件 ID', async () => {
    const shared = { code: EXCERPT.code, url: EXCERPT.url };
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json(shared, { status: 201 }));

    expect(await client(fetch).share(CATALOG, { ...BOUNDS, extra: 'x' } as typeof BOUNDS)).toEqual({
      ok: true,
      value: shared,
    });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/private-rooms/${CATALOG}/excerpts`),
      expect.objectContaining({
        body: JSON.stringify(BOUNDS),
        credentials: 'include',
        method: 'POST',
      }),
    );
  });

  it('读分享过的发 GET，拆掉外面那一层', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(
      Response.json({
        excerpts: [EXCERPT, { ...EXCERPT, code: 'Wd4Rt8Nx', url: null, mine: false }],
      }),
    );

    const result = await client(fetch).list(CATALOG);

    expect(result.ok ? result.value.map((excerpt) => excerpt.code) : result.error).toEqual([
      'Kq7Lm2Zp',
      'Wd4Rt8Nx',
    ]);
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/private-rooms/${CATALOG}/excerpts`),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
  });

  it('删掉发 DELETE 到这一段，回 204 就算成', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(new Response(null, { status: 204 }));

    expect(await client(fetch).remove(CATALOG, EXCERPT.code)).toEqual({
      ok: true,
      value: undefined,
    });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/private-rooms/${CATALOG}/excerpts/${EXCERPT.code}`),
      expect.objectContaining({ credentials: 'include', method: 'DELETE' }),
    );
  });

  it('回答多一个字段、少一个字段、段码不像样都当成无效', async () => {
    for (const body of [
      { excerpts: [{ ...EXCERPT, sharedBy: '@ada:agent-room.test' }] },
      { excerpts: [{ ...EXCERPT, mine: undefined }] },
      { excerpts: [{ ...EXCERPT, code: 'nope' }] },
      { excerpts: [], next: null },
    ]) {
      const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(body));
      const result = await client(fetch).list(CATALOG);
      expect(result.ok ? null : result.error.code).toBe('private_room.invalid_response');
    }
  });

  it('选的不对时把 details.reason 带出来，还有关联 ID', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(
      failure(400, {
        category: 'validation',
        code: 'room_excerpt.invalid_range',
        correlationId: 'corr-1',
        details: { reason: 'before_live' },
        extensions: {},
        message: '只能分享开了实况以后说的话。',
        retryable: false,
      }),
    );

    expect(await client(fetch).share(CATALOG, BOUNDS)).toEqual({
      ok: false,
      error: {
        code: 'room_excerpt.invalid_range',
        correlationId: 'corr-1',
        reason: 'before_live',
        retryable: false,
      },
    });
  });

  it('太多了（429）带出还要等几秒；正文没说就看 Retry-After', async () => {
    const inBody = vi.fn<typeof globalThis.fetch>().mockResolvedValue(
      failure(429, {
        code: 'room_excerpt.rate_limited',
        retryable: true,
        retryAfterSeconds: 1200,
      }),
    );
    const inHeader = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        failure(
          429,
          { code: 'room_excerpt.rate_limited', retryable: true },
          { 'Retry-After': '90' },
        ),
      );

    const fromBody = await client(inBody).share(CATALOG, BOUNDS);
    const fromHeader = await client(inHeader).share(CATALOG, BOUNDS);

    expect(fromBody.ok ? null : fromBody.error.retryAfterSeconds).toBe(1200);
    expect(fromHeader.ok ? null : fromHeader.error.retryAfterSeconds).toBe(90);
  });

  it('连不上说 private_room.unreachable，可以再试', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockRejectedValue(new TypeError('offline'));

    expect(await client(fetch).remove(CATALOG, EXCERPT.code)).toEqual({
      ok: false,
      error: { code: 'private_room.unreachable', retryable: true },
    });
  });
});
