import { describe, expect, it } from 'vitest';

import { installBuiltinPolyfills, type BuiltinScope } from './builtin-polyfills';

type AnySignal = (signals: Iterable<AbortSignal>) => AbortSignal;

/** 像 iOS 17.3 及以前的浏览器：没有 `Promise.withResolvers`、`AbortSignal.any`。 */
function oldBrowser(): BuiltinScope {
  class OldPromise<T> extends Promise<T> {}
  Reflect.defineProperty(OldPromise, 'withResolvers', {
    configurable: true,
    value: undefined,
    writable: true,
  });
  return { AbortController, AbortSignal: {}, Promise: OldPromise };
}

function anyOf(scope: BuiltinScope): AnySignal {
  const any = scope.AbortSignal.any;
  if (typeof any !== 'function') throw new Error('AbortSignal.any 没补上');
  return any as AnySignal;
}

describe('旧浏览器缺的内置函数', () => {
  it('补上 Promise.withResolvers，用调用它的构造函数，兑现和拒绝都传得过去', async () => {
    const scope = oldBrowser();
    installBuiltinPolyfills(scope);
    const OldPromise = scope.Promise as unknown as PromiseConstructor;

    const fulfilled = OldPromise.withResolvers<number>();
    expect(fulfilled.promise).toBeInstanceOf(OldPromise);
    fulfilled.resolve(7);
    await expect(fulfilled.promise).resolves.toBe(7);

    const rejected = OldPromise.withResolvers<number>();
    rejected.reject(new Error('停了'));
    await expect(rejected.promise).rejects.toThrow('停了');
  });

  it('补上 AbortSignal.any：已经停了的信号直接带着它的原因', () => {
    const scope = oldBrowser();
    installBuiltinPolyfills(scope);
    const stopped = new AbortController();
    stopped.abort('timeout');

    const combined = anyOf(scope)([new AbortController().signal, stopped.signal]);

    expect(combined.aborted).toBe(true);
    expect(combined.reason).toBe('timeout');
  });

  it('补上 AbortSignal.any：先停的那个说了算，后停的不改原因', () => {
    const scope = oldBrowser();
    installBuiltinPolyfills(scope);
    const session = new AbortController();
    const timeout = new AbortController();

    const combined = anyOf(scope)([session.signal, timeout.signal]);
    expect(combined.aborted).toBe(false);
    session.abort('signed out');
    timeout.abort('timeout');

    expect(combined.aborted).toBe(true);
    expect(combined.reason).toBe('signed out');
  });

  it('浏览器本来就有的不动', () => {
    const original = Object.getOwnPropertyDescriptor(Promise, 'withResolvers');
    const native: AnySignal = () => new AbortController().signal;
    const scope: BuiltinScope = { AbortController, AbortSignal: { any: native }, Promise };

    installBuiltinPolyfills(scope);

    expect(Object.getOwnPropertyDescriptor(Promise, 'withResolvers')).toEqual(original);
    expect(scope.AbortSignal.any).toBe(native);
  });
});
