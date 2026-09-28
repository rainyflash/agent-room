import { describe, expect, it, vi } from 'vitest';

import { MatrixCryptoStoreCleanup } from './matrix-crypto-store-cleanup';

const KEY = 'agent-room.matrix-crypto-cleanup.v1';
const FIRST = 'agent-room-crypto-0a1b2c3d';
const SECOND = 'agent-room-crypto-11223344';

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {
      return values.size;
    },
    clear: () => {
      values.clear();
    },
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => {
      values.delete(key);
    },
    setItem: (key, value) => {
      values.set(key, value);
    },
  };
}

type Outcome = 'success' | 'blocked' | 'error';

function fakeIndexedDB(outcome: (name: string) => Outcome) {
  const requested: string[] = [];
  const factory = {
    deleteDatabase(name: string) {
      requested.push(name);
      const request: { onsuccess?: () => void; onerror?: () => void; onblocked?: () => void } = {};
      queueMicrotask(() => {
        const result = outcome(name);
        if (result === 'success') request.onsuccess?.();
        else if (result === 'blocked') request.onblocked?.();
        else request.onerror?.();
      });
      return request;
    },
  } as unknown as IDBFactory;
  return { factory, requested };
}

function remembered(storage: Storage): unknown {
  return JSON.parse(storage.getItem(KEY) ?? '[]');
}

describe('MatrixCryptoStoreCleanup', () => {
  it('记下没删掉的加密库，删掉后不再记着，也不接受来历不明的名字', () => {
    const storage = memoryStorage();
    const cleanup = new MatrixCryptoStoreCleanup(storage, undefined);

    cleanup.defer(FIRST);
    cleanup.defer(FIRST);
    cleanup.defer(SECOND);
    cleanup.defer('some-other-database');
    expect(remembered(storage)).toEqual([FIRST, SECOND]);

    cleanup.done(FIRST);
    expect(remembered(storage)).toEqual([SECOND]);
    cleanup.done(SECOND);
    expect(storage.getItem(KEY)).toBeNull();
  });

  it('启动时删掉两份加密库，都删掉才不再记着；仍被占用就留到下次', async () => {
    const storage = memoryStorage();
    storage.setItem(KEY, JSON.stringify([FIRST, SECOND, 'not-ours']));
    const { factory, requested } = fakeIndexedDB((name) =>
      name.startsWith(SECOND) ? 'blocked' : 'success',
    );

    new MatrixCryptoStoreCleanup(storage, factory).retry();

    await vi.waitFor(() => {
      expect(remembered(storage)).toEqual([SECOND]);
    });
    expect(requested).toEqual([
      `${FIRST}::matrix-sdk-crypto`,
      `${SECOND}::matrix-sdk-crypto`,
      `${FIRST}::matrix-sdk-crypto-meta`,
    ]);
  });

  it('删除出错或没有 IndexedDB 时什么也不丢', async () => {
    const storage = memoryStorage();
    storage.setItem(KEY, JSON.stringify([FIRST]));
    const { factory, requested } = fakeIndexedDB(() => 'error');

    new MatrixCryptoStoreCleanup(storage, factory).retry();
    await vi.waitFor(() => {
      expect(requested).toEqual([`${FIRST}::matrix-sdk-crypto`]);
    });
    expect(remembered(storage)).toEqual([FIRST]);

    new MatrixCryptoStoreCleanup(storage, undefined).retry();
    expect(remembered(storage)).toEqual([FIRST]);
  });

  it('存储里的内容坏了就当没有', () => {
    const storage = memoryStorage();
    storage.setItem(KEY, '{broken');
    const cleanup = new MatrixCryptoStoreCleanup(storage, undefined);

    cleanup.defer(FIRST);
    expect(remembered(storage)).toEqual([FIRST]);
  });
});
