import { describe, expect, it, vi } from 'vitest';

import { ControlPlanePublicLobbyEntryClient } from '@/features/lobby-entry/adapters/control-plane-public-lobby-entry-client';

const target = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e46',
  matrixRoomId: '!public-lobby:matrix.agent-room.test',
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e47',
};

describe('控制面公开大厅入场适配器', () => {
  it('使用同源会话要求云端幂等确保权威房间', async () => {
    const fetch = vi.fn<typeof globalThis.fetch>().mockResolvedValue(Response.json(target));
    const gateway = new ControlPlanePublicLobbyEntryClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    await expect(gateway.resolve(target.catalogId)).resolves.toEqual({ ok: true, value: target });
    expect(fetch).toHaveBeenCalledWith(
      new URL(`https://api.agent-room.test/lobbies/${target.catalogId}/entry`),
      expect.objectContaining({ cache: 'no-store', credentials: 'include', method: 'POST' }),
    );
  });

  it('拒绝把目录编号或畸形 Matrix 房间当成可导航目标', async () => {
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(Response.json({ ...target, matrixRoomId: target.catalogId }));
    const gateway = new ControlPlanePublicLobbyEntryClient({
      baseUrl: 'https://api.agent-room.test',
      fetch,
    });

    await expect(gateway.resolve(target.catalogId)).resolves.toEqual({
      error: { code: 'lobby_entry.response_invalid', retryable: true },
      ok: false,
    });
  });
});
