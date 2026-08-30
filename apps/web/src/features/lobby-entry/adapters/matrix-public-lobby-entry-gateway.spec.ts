import { RoomEvent, type MatrixClient, type Room } from 'matrix-js-sdk';
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

  it('等待同步事件确认延迟收敛的成员关系', async () => {
    const projection = delayedRoom('invite');
    const client = {
      getRoom: () => projection.room,
      joinRoom: vi.fn().mockResolvedValue(projection.room),
    } as unknown as MatrixClient;
    const gateway = new MatrixSdkPublicLobbyEntryGateway(source(client), {
      joinConfirmationTimeoutMilliseconds: 1_000,
    });

    const joining = gateway.join(roomId);
    await vi.waitFor(() => expect(projection.listenerCount()).toBe(1));
    projection.publish('join');

    await expect(joining).resolves.toEqual({ ok: true, value: undefined });
    expect(projection.listenerCount()).toBe(0);
  });

  it('成员关系未在时限内收敛时失败关闭并清理监听器', async () => {
    const projection = delayedRoom('invite');
    const client = {
      getRoom: () => projection.room,
      joinRoom: vi.fn().mockResolvedValue(projection.room),
    } as unknown as MatrixClient;
    const gateway = new MatrixSdkPublicLobbyEntryGateway(source(client), {
      joinConfirmationTimeoutMilliseconds: 1,
    });

    await expect(gateway.join(roomId)).resolves.toEqual({
      error: { code: 'lobby_entry.matrix_join_unconfirmed', retryable: true },
      ok: false,
    });
    expect(projection.listenerCount()).toBe(0);
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

type Membership = ReturnType<Room['getMyMembership']>;
type MembershipListener = (room: Room, membership: Membership) => void;

function delayedRoom(initialMembership: Membership): {
  readonly listenerCount: () => number;
  readonly publish: (membership: Membership) => void;
  readonly room: Room;
} {
  let membership = initialMembership;
  const listeners = new Set<MembershipListener>();
  const candidate = {
    getMyMembership: () => membership,
    off: (event: RoomEvent, listener: MembershipListener) => {
      if (event === RoomEvent.MyMembership) listeners.delete(listener);
      return candidate;
    },
    on: (event: RoomEvent, listener: MembershipListener) => {
      if (event === RoomEvent.MyMembership) listeners.add(listener);
      return candidate;
    },
  };
  const matrixRoom = candidate as unknown as Room;
  return {
    listenerCount: () => listeners.size,
    publish: (nextMembership) => {
      membership = nextMembership;
      for (const listener of listeners) listener(matrixRoom, nextMembership);
    },
    room: matrixRoom,
  };
}
