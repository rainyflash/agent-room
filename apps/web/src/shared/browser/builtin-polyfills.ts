/**
 * 补上旧浏览器没有的内置函数。构建只把语法降到 Safari 16.4，不补内置函数；下面两个都要
 * iOS/Safari 17.4、Chrome 116 起才有：
 * - `Promise.withResolvers`：matrix-js-sdk 的加密库每次同步完要发请求时都用它。缺了就同步一次以后
 *   整个停住，这台设备的密钥一直传不上服务器、签不好，加密房间里一句也发不出去
 *   （2026-10-19 维护者的 iPhone，iOS 17.2）。
 * - `AbortSignal.any`：检查网页新版本时用它（会话请求那边自己合并两个信号，见 `session-request-scope.ts`）。
 *
 * 只在缺的时候补，已有的不动。`main.tsx` 第一个导入它，别的模块加载之前就补好。
 */

type ResolversPromise = {
  withResolvers?: unknown;
  new <T>(
    executor: (
      resolve: (value: T | PromiseLike<T>) => void,
      reject: (reason?: unknown) => void,
    ) => void,
  ): Promise<T>;
};

export type BuiltinScope = {
  readonly AbortController: new () => AbortController;
  readonly AbortSignal: { any?: unknown };
  readonly Promise: ResolversPromise;
};

export function installBuiltinPolyfills(scope: BuiltinScope = globalThis): void {
  if (typeof scope.Promise.withResolvers !== 'function') {
    define(scope.Promise, 'withResolvers', function withResolvers<T>(this: ResolversPromise) {
      let resolve!: (value: T | PromiseLike<T>) => void;
      let reject!: (reason?: unknown) => void;
      // 照规范用调用它的构造函数，`Promise.withResolvers()` 拿到的就是普通的 Promise。
      const promise = new this<T>((settle, fail) => {
        resolve = settle;
        reject = fail;
      });
      return { promise, reject, resolve };
    });
  }
  if (typeof scope.AbortSignal.any !== 'function') {
    const Controller = scope.AbortController;
    define(scope.AbortSignal, 'any', function any(signals: Iterable<AbortSignal>): AbortSignal {
      const combined = new Controller();
      const sources = [...signals];
      const early = sources.find((source) => source.aborted);
      if (early !== undefined) {
        combined.abort(early.reason);
        return combined.signal;
      }
      // 有一个停了就合起来停，同时摘掉挂在其余信号上的监听。
      const listening = new Controller();
      for (const source of sources) {
        source.addEventListener(
          'abort',
          () => {
            combined.abort(source.reason);
            listening.abort();
          },
          { once: true, signal: listening.signal },
        );
      }
      return combined.signal;
    });
  }
}

function define(target: object, name: string, value: unknown): void {
  Object.defineProperty(target, name, {
    configurable: true,
    enumerable: false,
    value,
    writable: true,
  });
}

installBuiltinPolyfills();
