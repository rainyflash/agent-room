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

  it('等待同步事件确认延迟收敛的成员关系', async () => {
    const projection = delayedProjection('invite');
    const client = {
      getRoom: () => projection.currentRoom(),
      joinRoom: vi.fn().mockResolvedValue(room('leave')),
    } as unknown as MatrixClient;
    const clients = reactiveSource(client);
    const gateway = new MatrixSdkPublicLobbyEntryGateway(clients.source, {
      joinConfirmationTimeoutMilliseconds: 1_000,
    });

    const joining = gateway.join(roomId);
    await vi.waitFor(() => {
      expect(clients.listenerCount()).toBe(1);
    });
    projection.replace('join');
    clients.publish();

    await expect(joining).resolves.toEqual({ ok: true, value: undefined });
    expect(clients.listenerCount()).toBe(0);
  });

  it('成员关系未在时限内收敛时失败关闭并清理监听器', async () => {
    const projection = delayedProjection('invite');
    const client = {
      getRoom: () => projection.currentRoom(),
      joinRoom: vi.fn().mockResolvedValue(room('leave')),
    } as unknown as MatrixClient;
    const clients = reactiveSource(client);
    const gateway = new MatrixSdkPublicLobbyEntryGateway(clients.source, {
      joinConfirmationTimeoutMilliseconds: 1,
    });

    await expect(gateway.join(roomId)).resolves.toEqual({
      error: { code: 'lobby_entry.matrix_join_unconfirmed', retryable: true },
      ok: false,
    });
    expect(clients.listenerCount()).toBe(0);
  });

  it('确认期间客户端租约被替换时立即失败关闭并清理监听器', async () => {
    const client = {
      getRoom: () => room('invite'),
      joinRoom: vi.fn().mockResolvedValue(room('leave')),
    } as unknown as MatrixClient;
    const clients = reactiveSource(client);
    const gateway = new MatrixSdkPublicLobbyEntryGateway(clients.source, {
      joinConfirmationTimeoutMilliseconds: 1_000,
    });

    const joining = gateway.join(roomId);
    await vi.waitFor(() => {
      expect(clients.listenerCount()).toBe(1);
    });
    clients.replace(null);

    await expect(joining).resolves.toEqual({
      error: { code: 'lobby_entry.matrix_join_unconfirmed', retryable: true },
      ok: false,
    });
    expect(clients.listenerCount()).toBe(0);
  });
});

function source(client: MatrixClient | null): MatrixClientSource {
  return {
    current: () => client,
    subscribe: () => () => undefined,
  };
}

function room(membership: ReturnType<Room['getMyMembership']>): Room {
  return { getMyMembership: () => membership } as unknown as Room;
}

function delayedProjection(initialMembership: ReturnType<Room['getMyMembership']>): {
  readonly currentRoom: () => Room;
  readonly replace: (membership: ReturnType<Room['getMyMembership']>) => void;
} {
  let currentRoom = room(initialMembership);
  return {
    currentRoom: () => currentRoom,
    replace: (membership) => {
      currentRoom = room(membership);
    },
  };
}

function reactiveSource(initialClient: MatrixClient | null): {
  readonly listenerCount: () => number;
  readonly publish: () => void;
  readonly replace: (client: MatrixClient | null) => void;
  readonly source: MatrixClientSource;
} {
  let client = initialClient;
  const listeners = new Set<() => void>();
  const publish = (): void => {
    for (const listener of listeners) listener();
  };
  return {
    listenerCount: () => listeners.size,
    publish,
    replace: (nextClient) => {
      client = nextClient;
      publish();
    },
    source: {
      current: () => client,
      subscribe: (listener) => {
        listeners.add(listener);
        return () => {
          listeners.delete(listener);
        };
      },
    },
  };
}
