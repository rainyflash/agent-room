import { describe, expect, it, vi } from 'vitest';

import { err, ok } from '@/shared/result';
import type { AgentOwnerLookupGateway, AgentOwnerRecord } from '../domain/agent-owners';
import { AgentOwnerStore } from './agent-owner-store';

const id = (index: number) => `01990d9e-8400-7000-8000-${String(index).padStart(12, '0')}`;
const room = '01990d9e-8400-7000-8000-000000000401';
const otherRoom = '01990d9e-8400-7000-8000-000000000402';
const ada = { displayName: 'Ada', principalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42' };

const local = (agentId: string): AgentOwnerRecord => ({ agentId, kind: 'local', owner: ada });

function harness(lookup: AgentOwnerLookupGateway['lookup']) {
  const tasks: (() => void)[] = [];
  const timers: { readonly at: number; readonly task: () => void }[] = [];
  let now = 1_000;
  const gateway = { lookup: vi.fn(lookup) };
  const store = new AgentOwnerStore(gateway, {
    now: () => now,
    schedule: (task) => tasks.push(task),
    retryAfterMs: 60_000,
    unansweredRetryMs: 30_000,
    later: (task, delayMs) => timers.push({ at: now + delayMs, task }),
  });
  const flush = async () => {
    for (const task of tasks.splice(0)) task();
    await Promise.resolve();
    await Promise.resolve();
  };
  return {
    gateway,
    store,
    flush,
    /** 往后拨时间，到点的“再问一次”照顺序跑。 */
    advance: async (millis: number) => {
      now += millis;
      const due = timers.filter((timer) => timer.at <= now);
      for (const timer of due) timers.splice(timers.indexOf(timer), 1);
      for (const timer of due) timer.task();
      await flush();
    },
    pendingTimers: () => timers.length,
  };
}

describe('Agent 是谁的', () => {
  it('同一个房间同一轮要问的攒成一批，问到的一直记着', async () => {
    const runtime = harness((_room, ids) => Promise.resolve(ok(ids.map(local))));
    const listener = vi.fn();
    runtime.store.subscribe(listener);

    runtime.store.request(room, [id(1), id(2)]);
    runtime.store.request(room, [id(2), id(3), 'not-an-agent-id']);
    await runtime.flush();

    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(1);
    expect(runtime.gateway.lookup).toHaveBeenCalledWith(room, [id(1), id(2), id(3)]);
    expect(runtime.store.owners(room).get(id(2))).toEqual({ kind: 'local', owner: ada });
    expect(listener).toHaveBeenCalledTimes(1);

    runtime.store.request(room, [id(1), id(2), id(3)]);
    await runtime.flush();
    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(1);
  });

  it('按房间分开问、分开记：服务器只回答此刻在那个房间里的', async () => {
    const runtime = harness((catalogId, ids) =>
      Promise.resolve(ok(catalogId === room ? ids.map(local) : [])),
    );

    runtime.store.request(room, [id(1)]);
    runtime.store.request(otherRoom, [id(1)]);
    await runtime.flush();

    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(2);
    expect(runtime.store.owners(room).has(id(1))).toBe(true);
    expect(runtime.store.owners(otherRoom).has(id(1))).toBe(false);
  });

  it('超过一百个时分批问', async () => {
    const runtime = harness(() => Promise.resolve(ok([])));

    runtime.store.request(
      room,
      Array.from({ length: 150 }, (_, index) => id(index + 1)),
    );
    await runtime.flush();

    expect(runtime.gateway.lookup.mock.calls.map(([, ids]) => ids.length)).toEqual([100, 50]);
  });

  it('没回答的（刚进来的 Agent）过一会儿自己再问，最多再问三次', async () => {
    const runtime = harness(() => Promise.resolve(ok([])));

    runtime.store.request(room, [id(1)]);
    await runtime.flush();
    // 还没到时候，名单再刷新也不问。
    runtime.store.request(room, [id(1)]);
    await runtime.flush();
    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(1);

    for (let round = 0; round < 5; round += 1) await runtime.advance(30_000);
    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(4);
    expect(runtime.pendingTimers()).toBe(0);

    // 名单变了、又过了再问的时间，照样会问。
    runtime.store.request(room, [id(1)]);
    await runtime.flush();
    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(5);
  });

  it('查失败的过一分钟再问，问到了就记下', async () => {
    let fail = true;
    const runtime = harness((_room, ids) =>
      Promise.resolve(fail ? err({ code: 'unavailable' as const }) : ok(ids.map(local))),
    );

    runtime.store.request(room, [id(1)]);
    await runtime.flush();
    expect(runtime.store.owners(room).size).toBe(0);

    fail = false;
    await runtime.advance(30_000);
    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(1);
    await runtime.advance(30_000);
    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(2);
    expect(runtime.store.owners(room).get(id(1))).toEqual({ kind: 'local', owner: ada });
  });

  it('退出登录时清空，还在路上的回答也不要', async () => {
    let answer: (value: Awaited<ReturnType<AgentOwnerLookupGateway['lookup']>>) => void = () =>
      undefined;
    const runtime = harness(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    runtime.store.request(room, [id(1)]);
    await runtime.flush();

    runtime.store.clear();
    answer(ok([local(id(1))]));
    await Promise.resolve();
    await Promise.resolve();

    expect(runtime.store.owners(room).size).toBe(0);
    expect(runtime.store.getSnapshot().size).toBe(0);
  });
});
