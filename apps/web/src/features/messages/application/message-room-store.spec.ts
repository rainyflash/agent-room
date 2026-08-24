import { describe, expect, it, vi } from 'vitest';

import { MessageRoomStore } from './message-room-store';
import type { MessageGateway, MessageReadResult } from '@/features/messages/domain/message';
import { err, ok } from '@/shared/result';

describe('MessageRoomStore', () => {
  it('共享单一源订阅并在最后一个观察者离开时释放', () => {
    const detach = vi.fn();
    const gateway = gatewayWith(ok(room()), detach);
    const store = new MessageRoomStore(gateway.value, '!public:agent-room.test');
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

  it('同步通知与显式重试都会重新读取权威时间线', () => {
    let notify = (): void => undefined;
    const read = vi
      .fn<() => MessageReadResult>()
      .mockReturnValueOnce(err({ code: 'messages.matrix_unavailable', retryable: true }))
      .mockReturnValue(ok(room()));
    const gateway: MessageGateway = {
      read,
      subscribe: (_roomId, listener) => {
        notify = listener;
        return noop;
      },
    };
    const store = new MessageRoomStore(gateway, '!public:agent-room.test');
    const listener = vi.fn();
    store.subscribe(listener);

    expect(store.getSnapshot()).toEqual({
      code: 'messages.matrix_unavailable',
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

function gatewayWith(result: MessageReadResult, detach: () => void) {
  const read = vi.fn(() => result);
  const subscribe = vi.fn(() => detach);
  return {
    read,
    subscribe,
    value: { read, subscribe } satisfies MessageGateway,
  };
}

function room() {
  return {
    messages: [],
    observedAtUnixMs: 1_700_000_000_000,
    roomId: '!public:agent-room.test',
  } as const;
}

function noop(): void {
  return undefined;
}
