// @vitest-environment jsdom

import type { MatrixPresenceObservation } from '@agent-room/protocol';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { AgentPresenceSource } from '@/features/lobby/adapters/matrix-presence-tracker';
import { useAgentPresence } from './use-agent-presence';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/** 像追踪器一样：读到的是内存里记下的，问到人时通知订阅者。 */
class FakePresence implements AgentPresenceSource {
  readonly known = new Map<string, MatrixPresenceObservation>();
  readonly asked: string[][] = [];
  readonly #listeners = new Set<() => void>();

  observe(userIds: readonly string[]): ReadonlyMap<string, MatrixPresenceObservation> {
    this.asked.push([...userIds]);
    return new Map(
      userIds.flatMap((id) => {
        const known = this.known.get(id);
        return known === undefined ? [] : [[id, known] as const];
      }),
    );
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  notify(): void {
    for (const listener of this.#listeners) listener();
  }
}

describe('我的 Agent 的在线状态', () => {
  it('问到了马上更新，同步里变了的隔一会儿再读到', () => {
    vi.useFakeTimers();
    const source = new FakePresence();
    const { result } = renderHook(() =>
      useAgentPresence(source, ['@b:test', '@a:test', '@a:test']),
    );
    expect(result.current.size).toBe(0);
    expect(source.asked[0]).toEqual(['@a:test', '@b:test']);

    source.known.set('@a:test', { state: 'online' });
    act(() => {
      source.notify();
    });
    expect(result.current.get('@a:test')?.state).toBe('online');

    source.known.set('@a:test', { offlineSeenAtUnixMs: 9_000, state: 'offline' });
    act(() => {
      vi.advanceTimersByTime(5_000);
    });
    expect(result.current.get('@a:test')).toEqual({ offlineSeenAtUnixMs: 9_000, state: 'offline' });
  });

  it('没变就还是同一份，不让页面白白重算', () => {
    vi.useFakeTimers();
    const source = new FakePresence();
    source.known.set('@a:test', { state: 'unavailable' });
    const { result } = renderHook(() => useAgentPresence(source, ['@a:test']));
    const first = result.current;
    act(() => {
      vi.advanceTimersByTime(15_000);
    });
    expect(result.current).toBe(first);
  });

  it('没有来源时是空的', () => {
    const { result } = renderHook(() => useAgentPresence(undefined, ['@a:test']));
    expect(result.current.size).toBe(0);
  });
});
