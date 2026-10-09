import { EventEmitter } from 'node:events';

import type { MatrixClient, MatrixEvent } from 'matrix-js-sdk';
import { describe, expect, it, vi } from 'vitest';

import { MatrixPresenceTracker } from './matrix-presence-tracker';
import { MatrixClientRegistry } from '@/shared/matrix/matrix-client-registry';

const START = 1_800_000_000_000;

describe('MatrixPresenceTracker', () => {
  it('同步里带来的在线状态直接用，不再去问', () => {
    const { client, tracker, clock } = setup();

    client.presence('@a:agent-room.test', { presence: 'online', last_active_ago: 5_000 });
    clock.advance(1_000);

    expect(tracker.observe(['@a:agent-room.test'])).toEqual(
      new Map([['@a:agent-room.test', { state: 'online', lastActiveAtUnixMs: START - 5_000 }]]),
    );
    expect(client.requests).toHaveLength(0);
  });

  it('没拿到的各问一次，同时最多问 4 个，问到了通知订阅者', async () => {
    const { client, tracker } = setup();
    const listener = vi.fn();
    tracker.subscribe(listener);
    const users = ['a', 'b', 'c', 'd', 'e', 'f'].map((name) => `@${name}:agent-room.test`);

    expect(tracker.observe(users).size).toBe(0);
    expect(client.requests.map((request) => request.userId)).toEqual(users.slice(0, 4));
    tracker.observe(users);
    expect(client.requests).toHaveLength(4);

    for (const request of client.requests.slice(0, 4)) request.resolve({ presence: 'offline' });
    await settle();
    expect(listener).toHaveBeenCalledTimes(4);
    expect(client.requests.map((request) => request.userId)).toEqual(users);

    for (const request of client.requests.slice(4)) request.resolve({ presence: 'unavailable' });
    await settle();
    const observed = tracker.observe(users);
    expect(observed.get('@a:agent-room.test')).toEqual({ state: 'offline' });
    expect(observed.get('@f:agent-room.test')).toEqual({ state: 'unavailable' });
    expect(client.requests).toHaveLength(6);
  });

  it('一直看着它不离线、同步说它离线了，离线从这一刻算', () => {
    const { client, tracker, clock } = setup();

    client.presence('@a:agent-room.test', { presence: 'online' });
    for (let sync = 0; sync < 10; sync += 1) {
      clock.advance(30_000);
      client.synced();
    }
    clock.advance(30_000);
    client.presence('@a:agent-room.test', { presence: 'offline', last_active_ago: 60_000 });

    expect(tracker.observe(['@a:agent-room.test']).get('@a:agent-room.test')).toEqual({
      state: 'offline',
      offlineSeenAtUnixMs: START + 330_000,
      lastActiveAtUnixMs: START + 270_000,
    });
  });

  it('隔了很久才同步回来说它离线（电脑睡着了），不算看到它离线的那一刻', () => {
    const { client, tracker, clock } = setup();

    client.presence('@a:agent-room.test', { presence: 'online' });
    client.synced();
    clock.advance(3_600_000);
    client.presence('@a:agent-room.test', { presence: 'offline', last_active_ago: 3_000_000 });

    expect(tracker.observe(['@a:agent-room.test']).get('@a:agent-room.test')).toEqual({
      state: 'offline',
      lastActiveAtUnixMs: START + 600_000,
    });
  });

  it('问到的离线不算看到它离线；后来又上线、再离线才算', async () => {
    const { client, tracker, clock } = setup();

    tracker.observe(['@a:agent-room.test']);
    client.requests[0]?.resolve({ presence: 'offline', last_active_ago: 90_000 });
    await settle();
    expect(tracker.observe(['@a:agent-room.test']).get('@a:agent-room.test')).toEqual({
      state: 'offline',
      lastActiveAtUnixMs: START - 90_000,
    });

    clock.advance(10_000);
    client.presence('@a:agent-room.test', { presence: 'unavailable' });
    clock.advance(10_000);
    client.presence('@a:agent-room.test', { presence: 'offline' });
    expect(tracker.observe(['@a:agent-room.test']).get('@a:agent-room.test')).toEqual({
      state: 'offline',
      offlineSeenAtUnixMs: START + 20_000,
    });
  });

  it('问的时候同步里来了新的，以同步的为准', async () => {
    const { client, tracker } = setup();

    tracker.observe(['@a:agent-room.test']);
    client.presence('@a:agent-room.test', { presence: 'online' });
    client.requests[0]?.resolve({ presence: 'offline' });
    await settle();

    expect(tracker.observe(['@a:agent-room.test']).get('@a:agent-room.test')?.state).toBe('online');
  });

  it('问失败了，一分钟以后才再问同一个人', async () => {
    const { client, tracker, clock } = setup();

    tracker.observe(['@a:agent-room.test']);
    client.requests[0]?.reject(new Error('M_FORBIDDEN'));
    await settle();
    tracker.observe(['@a:agent-room.test']);
    expect(client.requests).toHaveLength(1);

    clock.advance(60_000);
    tracker.observe(['@a:agent-room.test']);
    expect(client.requests).toHaveLength(2);
  });

  it('不认识的在线状态和缺字段的回答都不算数', async () => {
    const { client, tracker } = setup();

    client.presence('@a:agent-room.test', { presence: 'busy' });
    tracker.observe(['@b:agent-room.test']);
    client.requests[0]?.resolve({});
    await settle();

    expect(tracker.observe(['@a:agent-room.test', '@b:agent-room.test']).size).toBe(0);
  });

  it('换了账户从头开始，旧客户端的事件和没问完的结果都不算', async () => {
    const { client, registry, tracker } = setup();
    tracker.observe(['@a:agent-room.test']);
    client.presence('@b:agent-room.test', { presence: 'online' });

    const next = new FakeClient();
    registry.replace(next.asClient());
    client.requests[0]?.resolve({ presence: 'online' });
    client.presence('@c:agent-room.test', { presence: 'online' });
    await settle();

    const observed = tracker.observe([
      '@a:agent-room.test',
      '@b:agent-room.test',
      '@c:agent-room.test',
    ]);
    expect(observed.size).toBe(0);
    expect(next.requests.map((request) => request.userId)).toEqual([
      '@a:agent-room.test',
      '@b:agent-room.test',
      '@c:agent-room.test',
    ]);
    expect(client.listenerCount('event')).toBe(0);
    expect(client.listenerCount('sync')).toBe(0);
  });
});

type PendingRequest = {
  readonly userId: string;
  readonly resolve: (value: unknown) => void;
  readonly reject: (error: unknown) => void;
};

class FakeClient extends EventEmitter {
  readonly requests: PendingRequest[] = [];

  getPresence(userId: string): Promise<unknown> {
    return new Promise((resolve, reject) => {
      this.requests.push({ userId, resolve, reject });
    });
  }

  /** 同步里来了一条 `m.presence`。 */
  presence(sender: string, content: object): void {
    const event = {
      getContent: () => content,
      getSender: () => sender,
      getType: () => 'm.presence',
    } as unknown as MatrixEvent;
    this.emit('event', event);
  }

  /** 一次同步成功。 */
  synced(): void {
    this.emit('sync', 'SYNCING', 'SYNCING');
  }

  asClient(): MatrixClient {
    return this as unknown as MatrixClient;
  }
}

function setup() {
  let now = START;
  const client = new FakeClient();
  const registry = new MatrixClientRegistry();
  registry.replace(client.asClient());
  const tracker = new MatrixPresenceTracker(registry, { now: () => now });
  return {
    client,
    clock: {
      advance: (ms: number) => {
        now += ms;
      },
    },
    registry,
    tracker,
  };
}

/** 等手头的请求结果都处理完。 */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}
