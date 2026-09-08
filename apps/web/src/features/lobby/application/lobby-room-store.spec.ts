import { describe, expect, it, vi } from 'vitest';

import { LobbyRoomStore } from './lobby-room-store';
import type { LobbyGateway, LobbyReadResult } from '@/features/lobby/domain/lobby';
import { err, ok } from '@/shared/result';

describe('LobbyRoomStore', () => {
  it('安静房间仍推进过期时钟，取消订阅后停止定时读取', () => {
    vi.useFakeTimers();
    try {
      const read = vi.fn(() => ok({ ...room(), observedAtUnixMs: Date.now() }));
      const store = new LobbyRoomStore({ read, subscribe: () => () => undefined }, '!studio:test');
      const detach = store.subscribe(() => undefined);
      const first = store.getSnapshot();
      vi.advanceTimersByTime(5_000);
      expect(read).toHaveBeenCalledTimes(2);
      expect(store.getSnapshot()).not.toEqual(first);
      detach();
      vi.advanceTimersByTime(30_000);
      expect(read).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });
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
    const detach = store.subscribe(listener);

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
    detach();
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
