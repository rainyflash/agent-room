import { describe, expect, it, vi } from 'vitest';

import { LOBBY_IDLE_REFRESH_MS, LobbyRoomStore } from './lobby-room-store';
import type { LobbyAgent, LobbyGateway, LobbyReadResult } from '@/features/lobby/domain/lobby';
import { err, ok } from '@/shared/result';

describe('LobbyRoomStore', () => {
  it('安静房间照样定时重读，没变就不打扰订阅者，满一分钟才推进观察时刻，取消订阅后停止', () => {
    vi.useFakeTimers();
    try {
      const read = vi.fn(() => ok({ ...room(), observedAtUnixMs: Date.now() }));
      const store = new LobbyRoomStore({ read, subscribe: () => () => undefined }, '!studio:test');
      const listener = vi.fn();
      const detach = store.subscribe(listener);
      const first = store.getSnapshot();
      vi.advanceTimersByTime(5_000);
      expect(read).toHaveBeenCalledTimes(2);
      expect(store.getSnapshot()).toBe(first);
      expect(listener).toHaveBeenCalledOnce();
      vi.advanceTimersByTime(LOBBY_IDLE_REFRESH_MS - 5_000);
      expect(read).toHaveBeenCalledTimes(13);
      expect(store.getSnapshot()).not.toBe(first);
      expect(listener).toHaveBeenCalledTimes(2);
      detach();
      vi.advanceTimersByTime(30_000);
      expect(read).toHaveBeenCalledTimes(13);
    } finally {
      vi.useRealTimers();
    }
  });

  it('房间变了也沿用没变的 Agent 对象，界面只重绘变了的部分', () => {
    let notify = (): void => undefined;
    const first = agent('01990d9e-8400-7000-8000-00000000000a');
    const second = agent('01990d9e-8400-7000-8000-00000000000b');
    const read = vi
      .fn<() => LobbyReadResult>()
      .mockReturnValueOnce(ok({ ...room(), agents: [first, second] }))
      .mockReturnValueOnce(
        ok({ ...room(), agents: [{ ...first }, { ...second, status: 'working' }] }),
      )
      .mockReturnValue(ok({ ...room(), agents: [{ ...first }, { ...second, status: 'working' }] }));
    const store = new LobbyRoomStore(
      {
        read,
        subscribe: (_roomId, listener) => {
          notify = listener;
          return () => undefined;
        },
      },
      '!public:agent-room.test',
    );
    const listener = vi.fn();
    store.subscribe(listener);
    const before = store.getSnapshot();

    notify();
    const after = store.getSnapshot();
    if (before.kind !== 'ready' || after.kind !== 'ready') throw new Error('房间应当已就绪');
    expect(after).not.toBe(before);
    expect(after.room.agents[0]).toBe(before.room.agents[0]);
    expect(after.room.agents[1]).not.toBe(before.room.agents[1]);
    expect(after.room.agents[1]?.status).toBe('working');

    notify();
    expect(store.getSnapshot()).toBe(after);
    expect(listener).toHaveBeenCalledTimes(2);
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

  it('源事件与显式重试都会重新读取真实状态，结果没变时不重复通知', () => {
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
    expect(listener).toHaveBeenCalledTimes(2);
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

function agent(agentId: string): LobbyAgent {
  return {
    agentId,
    displayName: '构建助手',
    instanceIds: [agentId],
    matrixUserId: '@build-agent:agent-room.test',
    status: 'idle',
    statusExpiresAtUnixMs: 1_700_000_300_000,
    trust: 'unknown',
    visibility: 'coarse',
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
