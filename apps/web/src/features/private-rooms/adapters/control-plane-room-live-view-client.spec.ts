import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneRoomLiveViewClient } from './control-plane-room-live-view-client';

const BASE = 'https://control.agent-room.test';
const CATALOG = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';

const LIVE = {
  canChange: true,
  enabled: true,
  enabledAtUnixMs: 1_790_000_000_000,
  live: true,
  unmetConditions: [],
};

function client(fetch: typeof globalThis.fetch) {
  return new ControlPlaneRoomLiveViewClient({ baseUrl: BASE, fetch });
}

describe('ControlPlaneRoomLiveViewClient', () => {
  it('读开关带着登录发 GET，回答照新的 schema 严格校验', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(LIVE));

    const result = await client(fetch).liveView(CATALOG);

    expect(result).toEqual({ ok: true, value: LIVE });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/private-rooms/${CATALOG}/live-view`),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
  });

  it('开关发 PUT，正文只有 enabled', async () => {
    const off = { ...LIVE, enabled: false, enabledAtUnixMs: null, live: false };
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(off));

    const result = await client(fetch).updateLiveView(CATALOG, false);

    expect(result).toEqual({ ok: true, value: off });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`${BASE}/private-rooms/${CATALOG}/live-view`),
      expect.objectContaining({
        body: JSON.stringify({ enabled: false }),
        credentials: 'include',
        method: 'PUT',
      }),
    );
  });

  it('差的几样照控制面的名字认，几样都能一起来', async () => {
    const blocked = {
      ...LIVE,
      enabled: false,
      enabledAtUnixMs: null,
      live: false,
      unmetConditions: [
        'sub_room',
        'closed',
        'not_listed',
        'platform_hold',
        'encrypted',
        'join_mode',
      ],
    };
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(blocked));

    expect(await client(fetch).liveView(CATALOG)).toEqual({ ok: true, value: blocked });
  });

  it('多一个字段、少一个字段、不认得的条件都当成回答无效', async () => {
    for (const body of [
      { ...LIVE, extra: true },
      { ...LIVE, canChange: undefined },
      { ...LIVE, unmetConditions: ['moon_phase'] },
      { ...LIVE, enabledAtUnixMs: -1 },
    ]) {
      const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(body));
      const result = await client(fetch).liveView(CATALOG);
      expect(result.ok ? null : result.error.code).toBe('private_room.invalid_response');
    }
  });

  it('开不了（409）时把 details.condition 带出来，还有关联 ID', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(
      Response.json(
        {
          category: 'conflict',
          code: 'room_live.not_eligible',
          correlationId: 'corr-1',
          details: { condition: 'join_mode' },
          extensions: {},
          message: '只有直接进的房间能放实况，要申请、只能邀请的房间不能放。',
          retryable: false,
        },
        { status: 409 },
      ),
    );

    const result = await client(fetch).updateLiveView(CATALOG, true);

    expect(result).toEqual({
      ok: false,
      error: {
        code: 'room_live.not_eligible',
        condition: 'join_mode',
        correlationId: 'corr-1',
        retryable: false,
      },
    });
  });

  it('别的错误没有 condition；details 不像样也照样读出错误码', async () => {
    for (const details of [{}, 'oops', { condition: 7 }]) {
      const fetch = vi
        .fn<typeof globalThis.fetch>()
        .mockResolvedValue(
          Response.json(
            { code: 'room_live.forbidden', details, retryable: false },
            { status: 403 },
          ),
        );
      const result = await client(fetch).updateLiveView(CATALOG, true);
      expect(result).toEqual({
        ok: false,
        error: { code: 'room_live.forbidden', retryable: false },
      });
    }
  });
});
