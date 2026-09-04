import { describe, expect, it, vi } from 'vitest';

import { ControlPlanePublicRoomDirectoryClient } from '@/features/room-directory/adapters/control-plane-public-room-directory-client';

const room = {
  activeInstanceCount: 1,
  catalogId: '0198b601-77a2-7f41-b4f4-940f291951b8',
  description: 'Shared Agent room',
  language: 'en',
  name: 'Public room',
  onlineAgentCount: 2,
  slug: 'public',
};

describe('控制面公共房间目录适配器', () => {
  it('使用同源会话读取权威房间目录', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ lobbies: [room] }));
    const client = new ControlPlanePublicRoomDirectoryClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    await expect(client.list()).resolves.toEqual({ ok: true, value: [room] });
    expect(fetch).toHaveBeenCalledWith(
      new URL('https://api.agent-room.test/lobbies/public'),
      expect.objectContaining({ credentials: 'include', method: 'GET' }),
    );
  });

  it('拒绝畸形目录而不把脏数据交给 UI', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ lobbies: [{ ...room, catalogId: 'not-a-uuid' }] }));
    const client = new ControlPlanePublicRoomDirectoryClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    await expect(client.list()).resolves.toEqual({
      error: { code: 'room_directory.response_invalid', retryable: true },
      ok: false,
    });
  });
});
