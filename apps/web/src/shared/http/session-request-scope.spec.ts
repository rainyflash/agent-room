import { describe, expect, it } from 'vitest';

import { SessionRequestScope } from './session-request-scope';

function pendingFetch(signals: AbortSignal[]): typeof globalThis.fetch {
  return (_input, init) =>
    new Promise<Response>((_resolve, reject) => {
      const signal = init?.signal;
      if (signal === undefined || signal === null) throw new Error('Missing request signal');
      signals.push(signal);
      signal.throwIfAborted();
      signal.addEventListener(
        'abort',
        () => {
          const reason: unknown = signal.reason;
          reject(
            reason instanceof Error || reason instanceof DOMException
              ? reason
              : new Error('Request cancelled'),
          );
        },
        { once: true },
      );
    });
}

describe('会话 HTTP 请求生命周期', () => {
  it('清理会话取消全部旧请求，新的会话请求仍可继续', async () => {
    const signals: AbortSignal[] = [];
    const scope = new SessionRequestScope(pendingFetch(signals));
    const first = expect(scope.fetch('https://api.test/agents')).rejects.toMatchObject({
      name: 'AbortError',
    });
    const second = expect(scope.fetch('https://api.test/agent-instances')).rejects.toMatchObject({
      name: 'AbortError',
    });
    scope.clear();
    await Promise.all([first, second]);
    const next = expect(scope.fetch('https://api.test/agents')).rejects.toMatchObject({
      name: 'AbortError',
    });
    expect(signals[2]?.aborted).toBe(false);
    scope.clear();
    await next;
  });

  it('调用方取消只影响自己的请求，不取消同一会话的其他请求', async () => {
    const signals: AbortSignal[] = [];
    const scope = new SessionRequestScope(pendingFetch(signals));
    const caller = new AbortController();
    const first = expect(
      scope.fetch('https://api.test/agents', { signal: caller.signal }),
    ).rejects.toMatchObject({ name: 'AbortError' });
    const second = expect(scope.fetch('https://api.test/agent-instances')).rejects.toMatchObject({
      name: 'AbortError',
    });
    caller.abort();
    await first;
    expect(signals[1]?.aborted).toBe(false);
    scope.clear();
    await second;
  });

  it('浏览器没有 AbortSignal.any 时请求照样发出，两边取消都生效', async () => {
    const original = Object.getOwnPropertyDescriptor(AbortSignal, 'any');
    Reflect.deleteProperty(AbortSignal, 'any');
    try {
      const signals: AbortSignal[] = [];
      const scope = new SessionRequestScope(pendingFetch(signals));
      const caller = new AbortController();
      const callerCancelled = expect(
        scope.fetch('https://api.test/lobbies/public', { signal: caller.signal }),
      ).rejects.toMatchObject({ name: 'AbortError' });
      const sessionCancelled = expect(
        scope.fetch('https://api.test/private-rooms', { signal: new AbortController().signal }),
      ).rejects.toMatchObject({ name: 'AbortError' });
      expect(signals).toHaveLength(2);
      caller.abort();
      await callerCancelled;
      expect(signals[1]?.aborted).toBe(false);
      scope.clear();
      await sessionCancelled;

      const finished = new AbortController();
      const resolved = new SessionRequestScope(() => Promise.resolve(new Response('{}')));
      await expect(
        resolved.fetch('https://api.test/lobbies/public', { signal: finished.signal }),
      ).resolves.toBeInstanceOf(Response);
      const alreadyCancelled = new AbortController();
      alreadyCancelled.abort();
      const late = new SessionRequestScope(pendingFetch(signals));
      await expect(
        late.fetch('https://api.test/lobbies/public', { signal: alreadyCancelled.signal }),
      ).rejects.toMatchObject({ name: 'AbortError' });
    } finally {
      if (original !== undefined) Object.defineProperty(AbortSignal, 'any', original);
    }
  });

  it('保留 Request 对象的取消信号，并允许 init 显式覆盖', async () => {
    const signals: AbortSignal[] = [];
    const scope = new SessionRequestScope(pendingFetch(signals));
    const caller = new AbortController();
    const request = new Request('https://api.test/agents', { signal: caller.signal });
    const inherited = expect(scope.fetch(request)).rejects.toMatchObject({ name: 'AbortError' });
    const overridden = expect(scope.fetch(request, { signal: null })).rejects.toMatchObject({
      name: 'AbortError',
    });
    caller.abort();
    await inherited;
    expect(signals[1]?.aborted).toBe(false);
    scope.clear();
    await overridden;
  });
});
