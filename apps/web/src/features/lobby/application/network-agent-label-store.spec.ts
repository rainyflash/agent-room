import { describe, expect, it, vi } from 'vitest';

import { err, ok } from '@/shared/result';
import type { NetworkAgentLookupGateway } from '../domain/network-agent-labels';
import { NetworkAgentLabelStore } from './network-agent-label-store';

const id = (index: number) => `01990d9e-8400-7000-8000-${String(index).padStart(12, '0')}`;

function harness(lookup: NetworkAgentLookupGateway['lookup']) {
  const tasks: (() => void)[] = [];
  let now = 1_000;
  const gateway = { lookup: vi.fn(lookup) };
  const store = new NetworkAgentLabelStore(gateway, {
    now: () => now,
    schedule: (task) => tasks.push(task),
    retryAfterMs: 60_000,
  });
  return {
    gateway,
    store,
    advance: (millis: number) => {
      now += millis;
    },
    flush: async () => {
      for (const task of tasks.splice(0)) task();
      await Promise.resolve();
      await Promise.resolve();
    },
  };
}

describe('网络 Agent 标记缓存', () => {
  it('同一轮要问的攒成一批，只有查到网络 Agent 时才通知', async () => {
    const runtime = harness((ids) => Promise.resolve(ok(new Set(ids.filter((x) => x === id(2))))));
    const listener = vi.fn();
    runtime.store.subscribe(listener);

    runtime.store.request([id(1), id(2)]);
    runtime.store.request([id(2), id(3), 'not-an-agent-id']);
    await runtime.flush();

    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(1);
    expect(runtime.gateway.lookup).toHaveBeenCalledWith([id(1), id(2), id(3)]);
    expect(runtime.store.getSnapshot()).toEqual(new Set([id(2)]));
    expect(listener).toHaveBeenCalledTimes(1);

    runtime.store.request([id(1), id(2), id(3)]);
    await runtime.flush();
    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(1);
  });

  it('超过一百个时分批问', async () => {
    const runtime = harness(() => Promise.resolve(ok(new Set<string>())));

    runtime.store.request(Array.from({ length: 150 }, (_, index) => id(index)));
    await runtime.flush();

    expect(runtime.gateway.lookup.mock.calls.map(([ids]) => ids.length)).toEqual([100, 50]);
  });

  it('查失败的过一会儿再问，其间不重复打扰服务器', async () => {
    const runtime = harness(() => Promise.resolve(err({ code: 'unavailable' as const })));

    runtime.store.request([id(1)]);
    await runtime.flush();
    runtime.store.request([id(1)]);
    await runtime.flush();
    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(1);
    expect(runtime.store.getSnapshot().size).toBe(0);

    runtime.advance(60_001);
    runtime.store.request([id(1)]);
    await runtime.flush();
    expect(runtime.gateway.lookup).toHaveBeenCalledTimes(2);
  });
});
