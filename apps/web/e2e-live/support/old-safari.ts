import type { Page } from '@playwright/test';

/**
 * 像 iOS/Safari 17.2 那样：页面脚本跑之前，拿掉它还没有、验收用的 Chromium 已经有的内置函数和接口。
 * 名单按 MDN 的兼容数据挑（Safari on iOS 的 `version_added` 晚于 17.2 或者没有）。新版 Chrome 自带的
 * 函数多了，库里“有就用、没有就退回”的那条退回的路只有旧 iPhone 走得到，CI 里一直没跑过。
 */
export async function removeBuiltinsMissingOnIos172(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const drop = (target: object | undefined, names: readonly PropertyKey[]): void => {
      if (target === undefined) return;
      for (const name of names) {
        try {
          Reflect.deleteProperty(target, name);
        } catch {
          // 拿不掉的（比如不可配置的）就留着。
        }
      }
    };
    const iteratorPrototype = Object.getPrototypeOf(
      Object.getPrototypeOf([][Symbol.iterator]()),
    ) as object;
    // Safari 17.4
    drop(Promise, ['withResolvers']);
    drop(AbortSignal, ['any']);
    drop(Object, ['groupBy']);
    drop(Map, ['groupBy']);
    drop(ArrayBuffer.prototype, ['transfer', 'transferToFixedLength', 'detached']);
    drop(Element.prototype, ['checkVisibility']);
    // Safari 18 及以后
    drop(Promise, ['try']);
    drop(URL, ['parse']);
    drop(RegExp, ['escape']);
    drop(Error, ['isError']);
    drop(Uint8Array, ['fromBase64', 'fromHex']);
    drop(Uint8Array.prototype, ['toBase64', 'toHex', 'setFromBase64', 'setFromHex']);
    drop(iteratorPrototype, [
      'map',
      'filter',
      'take',
      'drop',
      'flatMap',
      'reduce',
      'toArray',
      'forEach',
      'some',
      'every',
      'find',
    ]);
    drop(Math, ['f16round', 'sumPrecise']);
    drop(DataView.prototype, ['getFloat16', 'setFloat16']);
    drop(Atomics, ['pause']);
    drop(JSON, ['rawJSON', 'isRawJSON']);
    drop(Blob.prototype, ['bytes']);
    drop(Response.prototype, ['bytes']);
    drop(Request.prototype, ['bytes']);
    drop(Document.prototype, ['startViewTransition']);
    // Safari 一直没有
    drop(ReadableStream, ['from']);
    drop(ReadableStream.prototype, ['values', Symbol.asyncIterator]);
    drop(Navigator.prototype, ['connection', 'deviceMemory']);
    drop(globalThis, [
      'Iterator',
      'Float16Array',
      'DisposableStack',
      'AsyncDisposableStack',
      'requestIdleCallback',
      'cancelIdleCallback',
      'scheduler',
    ]);
  });
}
