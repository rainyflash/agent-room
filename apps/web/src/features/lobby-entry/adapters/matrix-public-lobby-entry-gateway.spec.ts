import type { MatrixClient, Room } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { MatrixSdkPublicLobbyEntryGateway } from '@/features/lobby-entry/adapters/matrix-public-lobby-entry-gateway';
import type { MatrixClientSource } from '@/shared/matrix/matrix-client-registry';

const roomId = '!public-lobby:matrix.agent-room.test';

describe('Matrix 公开大厅入场适配器', () => {
  it('没有当前 Matrix 客户端时失败关闭', async () => {
    const gateway = new MatrixSdkPublicLobbyEntryGateway(source(null));

    await expect(gateway.join(roomId)).resolves.toEqual({
      error: { code: 'lobby_entry.matrix_unavailable', retryable: true },
      ok: false,
    });
  });

  it('已加入房间时保持幂等且不重复调用 joinRoom', async () => {
    const joinRoom = vi.fn();
    const client = {
      getRoom: () => room('join'),
      joinRoom,
    } as unknown as MatrixClient;
    const gateway = new MatrixSdkPublicLobbyEntryGateway(source(client));

    await expect(gateway.join(roomId)).resolves.toEqual({ ok: true, value: undefined });
    expect(joinRoom).not.toHaveBeenCalled();
  });

  it('只有 Matrix 返回已加入成员关系后才报告成功', async () => {
    const joinRoom = vi.fn().mockResolvedValue(room('join'));
    const client = {
      getRoom: () => null,
      joinRoom,
    } as unknown as MatrixClient;
    const gateway = new MatrixSdkPublicLobbyEntryGateway(source(client));

    await expect(gateway.join(roomId)).resolves.toEqual({ ok: true, value: undefined });
    expect(joinRoom).toHaveBeenCalledWith(roomId);
  });
});

function source(client: MatrixClient | null): MatrixClientSource {
  return {
    current: () => client,
    subscribe: () => () => undefined,
  };
}

function room(membership: 'invite' | 'join'): Room {
  return { getMyMembership: () => membership } as unknown as Room;
}
