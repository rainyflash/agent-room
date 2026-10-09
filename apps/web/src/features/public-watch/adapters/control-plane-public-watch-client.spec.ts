import { describe, expect, it, vi } from 'vitest';

import { ControlPlanePublicWatchClient } from '@/features/public-watch/adapters/control-plane-public-watch-client';
import type { PublicWatch } from '@/features/public-watch/domain/public-watch';

const watch: PublicWatch = {
  schemaVersion: 1,
  lobby: {
    catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
    name: 'Agent Room Global',
    slug: 'agent-room-global',
  },
  participants: [
    { key: 'pq3T0vXw1aBc', name: 'Sol', kind: 'networkAgent', online: true, status: 'idle' },
    { key: 'pZ8kLm2nOpQr', name: 'Lin', kind: 'person', online: false, status: null },
  ],
  messages: [
    {
      key: 'mA1b2C3d4E5f',
      author: 'pq3T0vXw1aBc',
      text: 'Hello, I am Sol.',
      truncated: false,
      withheld: false,
      attachment: false,
      edited: false,
      replyTo: null,
      sentAtUnixMs: 1_792_483_200_000,
    },
  ],
  updatedAtUnixMs: 1_792_483_203_000,
};

function client(fetch: typeof globalThis.fetch) {
  return new ControlPlanePublicWatchClient({
    baseUrl: 'https://app.agent-room.test/_agent-room/api',
    fetch,
  });
}

describe('不登录看公共大厅的接口', () => {
  it('不带登录问控制面，地址保留同源前缀', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(watch));

    await expect(client(fetch).read('default')).resolves.toEqual({ ok: true, value: watch });
    expect(fetch).toHaveBeenCalledWith(
      new URL('https://app.agent-room.test/_agent-room/api/public-lobbies/default/watch'),
      expect.objectContaining({ credentials: 'omit', method: 'GET' }),
    );
  });

  it('回答里多了不认识的字段就不交给界面', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json({ ...watch, messages: [{ ...watch.messages[0], roomId: '!x:matrix' }] }),
      );

    await expect(client(fetch).read('default')).resolves.toEqual({
      error: { code: 'public_watch.response_invalid', retryable: true },
      ok: false,
    });
  });

  it('照实说没开放、没有这个大厅、暂时看不了', async () => {
    const answer = (status: number, code: string) =>
      vi
        .fn<typeof globalThis.fetch>()
        .mockResolvedValue(
          Response.json(
            { category: 'validation', code, correlationId: 'c', details: {}, message: 'm' },
            { status },
          ),
        );

    await expect(client(answer(404, 'public_watch.disabled')).read('default')).resolves.toEqual({
      error: { code: 'public_watch.disabled', retryable: false },
      ok: false,
    });
    await expect(
      client(answer(404, 'public_watch.lobby_not_found')).read('night-owls'),
    ).resolves.toEqual({
      error: { code: 'public_watch.lobby_not_found', retryable: false },
      ok: false,
    });
    await expect(client(answer(503, 'public_watch.unavailable')).read('default')).resolves.toEqual({
      error: { code: 'public_watch.unavailable', retryable: true },
      ok: false,
    });
  });

  it('不像 slug 的不去问服务器', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>();

    await expect(client(fetch).read('../auth/session')).resolves.toEqual({
      error: { code: 'public_watch.lobby_not_found', retryable: false },
      ok: false,
    });
    expect(fetch).not.toHaveBeenCalled();
  });

  it('连不上时可以重试', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockRejectedValue(new TypeError('offline'));

    await expect(client(fetch).read('default')).resolves.toEqual({
      error: { code: 'public_watch.unreachable', retryable: true },
      ok: false,
    });
  });
});
