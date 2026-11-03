import { describe, expect, it, vi } from 'vitest';

import { ControlPlaneExploreClient } from '@/features/explore/adapters/control-plane-explore-client';
import { defaultExploreFilter, type ExploreRoom } from '@/features/explore/domain/explore';

const room: ExploreRoom = {
  agentCount: 2,
  catalogId: '0198c000-0000-7000-8000-000000000001',
  createdAtUnixMs: 1_792_483_200_000,
  description: 'Ask about borrow checker errors.',
  endToEndEncrypted: false,
  joinMode: 'open',
  lastMessageAtUnixMs: null,
  memberCount: 5,
  name: 'Rust help desk',
  ownerName: 'Grace',
  shareCode: 'aB3xYz',
  shareUrl: 'https://agentroom.test/r/aB3xYz',
  viewer: null,
};

function client(fetch: typeof globalThis.fetch) {
  return new ControlPlaneExploreClient({
    baseUrl: 'https://app.agent-room.test/_agent-room/api',
    fetch,
  });
}

describe('发现页的接口', () => {
  it('带上 Cookie 读目录，筛选和翻页写进查询参数，下一页的位置原样交回', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ nextCursor: '20', rooms: [room] }));

    await expect(
      client(fetch).browse({ q: ' rust ', quick: 'request', sort: 'members' }, '20'),
    ).resolves.toEqual({ ok: true, value: { nextCursor: '20', rooms: [room] } });
    expect(fetch).toHaveBeenCalledWith(
      new URL(
        'https://app.agent-room.test/_agent-room/api/public-rooms?q=rust&join=request&sort=members&cursor=20',
      ),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
  });

  it('最后一页没有下一页的位置', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json({ rooms: [] }));

    await expect(client(fetch).browse(defaultExploreFilter, null)).resolves.toEqual({
      ok: true,
      value: { nextCursor: null, rooms: [] },
    });
    expect(fetch.mock.calls[0]?.[0]).toEqual(
      new URL('https://app.agent-room.test/_agent-room/api/public-rooms'),
    );
  });

  it('回答里多了不认识的字段就不交给界面', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ rooms: [{ ...room, matrixRoomId: '!x:matrix' }] }));

    await expect(client(fetch).browse(defaultExploreFilter, null)).resolves.toEqual({
      error: { code: 'public_room.invalid_response', retryable: false },
      ok: false,
    });
  });

  it('被限流时带上过多少秒再试', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json(
          { code: 'public_rooms.rate_limited', correlationId: 'c-1' },
          { headers: { 'Retry-After': '30' }, status: 429 },
        ),
      );

    await expect(client(fetch).browse(defaultExploreFilter, null)).resolves.toEqual({
      error: {
        code: 'public_rooms.rate_limited',
        correlationId: 'c-1',
        retryAfterSeconds: 30,
        retryable: true,
      },
      ok: false,
    });
  });

  it('连不上时可以重试', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockRejectedValue(new TypeError('offline'));

    await expect(client(fetch).browse(defaultExploreFilter, null)).resolves.toEqual({
      error: { code: 'public_room.unreachable', retryable: true },
      ok: false,
    });
  });

  it('详情多几位管理员和在线人数；不像分享码的不去问服务器', async () => {
    const detail = { ...room, admins: ['Lin'], online: { atLeast: false, count: 3 } };
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(detail));

    await expect(client(fetch).detail('aB3xYz')).resolves.toEqual({ ok: true, value: detail });
    expect(fetch.mock.calls[0]?.[0]).toEqual(
      new URL('https://app.agent-room.test/_agent-room/api/public-rooms/aB3xYz'),
    );
    await expect(client(fetch).detail('../me')).resolves.toEqual({
      error: { code: 'public_room.not_found', retryable: false },
      ok: false,
    });
    expect(fetch).toHaveBeenCalledTimes(1);
  });
});
