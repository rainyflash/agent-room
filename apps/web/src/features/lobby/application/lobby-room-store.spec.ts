import { describe, expect, it, vi } from 'vitest';

import { LobbyRoomStore } from './lobby-room-store';
import type { LobbyGateway, LobbyReadResult } from '@/features/lobby/domain/lobby';
import { err, ok } from '@/shared/result';

describe('LobbyRoomStore', () => {
  it('首个订阅者建立单一源订阅，最后一个离开时释放', () => {
    const detach = vi.fn();
    const gateway = gatewayWith(ok(room()), detach);
    const store = new LobbyRoomStore(gateway.value, '!public:agent-room.test');
    const first = vi.fn();
    const second = vi.fn();

    const unsubscribeFirst = store.subscribe(first);
    const unsubscribeSecond = store.subscribe(second);
    unsubscribeFirst();
    unsubscribeSecond();

    expect(gateway.subscribe).toHaveBeenCalledOnce();
    expect(gateway.read).toHaveBeenCalledOnce();
    expect(first).toHaveBeenCalledOnce();
    expect(second).not.toHaveBeenCalled();
    expect(detach).toHaveBeenCalledOnce();
    expect(store.getSnapshot()).toEqual({ kind: 'ready', room: room() });
  });

  it('源事件与显式重试都会重新读取真实状态', () => {
    let notify = (): void => undefined;
    const read = vi
      .fn<() => LobbyReadResult>()
      .mockReturnValueOnce(err({ code: 'lobby.matrix_unavailable', retryable: true }))
      .mockReturnValue(ok(room()));
    const gateway: LobbyGateway = {
      read,
      subscribe: (_roomId, listener) => {
        notify = listener;
        return () => undefined;
      },
    };
    const store = new LobbyRoomStore(gateway, '!public:agent-room.test');
    const listener = vi.fn();
    store.subscribe(listener);

    expect(store.getSnapshot()).toEqual({
      code: 'lobby.matrix_unavailable',
      kind: 'failed',
      retryable: true,
    });

    notify();
    store.retry();

    expect(read).toHaveBeenCalledTimes(3);
    expect(listener).toHaveBeenCalledTimes(3);
    expect(store.getSnapshot()).toEqual({ kind: 'ready', room: room() });
  });
});

function gatewayWith(result: LobbyReadResult, detach: () => void) {
  const read = vi.fn(() => result);
  const subscribe = vi.fn(() => detach);
  return {
    read,
    subscribe,
    value: { read, subscribe } satisfies LobbyGateway,
  };
}

function room() {
  return {
    agents: [],
    name: '公开大厅',
    observedAtUnixMs: 1_700_000_000_000,
    roomId: '!public:agent-room.test',
  } as const;
}
