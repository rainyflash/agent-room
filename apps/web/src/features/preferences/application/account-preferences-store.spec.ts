import { describe, expect, it, vi } from 'vitest';

import { AccountPreferencesStore } from './account-preferences-store';
import {
  createAccountPreferencesDocument,
  mergeAccountPreferencesDocuments,
  updateAccountPreference,
  valuesFromAccountPreferences,
  type AccountPreferenceValues,
  type AccountPreferencesDocument,
} from '@/features/preferences/domain/account-preferences';
import type {
  AccountPreferencesGateway,
  AccountPreferencesScope,
  AccountPreferencesSyncFailure,
} from '@/features/preferences/domain/account-preferences-gateway';
import { err, ok, type Result } from '@/shared/result';

describe('账户偏好同步仓库', () => {
  it('首次同步采用服务端事实，不用设备本地默认覆盖已有账户', async () => {
    const gateway = new FakePreferencesGateway();
    gateway.remote = updated(document('DEVICE_B'), 'language', 'zh-CN', 'DEVICE_B');
    const store = new AccountPreferencesStore(gateway, {
      language: 'en',
      lobbyView: 'list',
    });

    const unsubscribe = store.subscribe(vi.fn());
    await expectSnapshot(store, 'synced');

    expect(store.getSnapshot()).toEqual({
      failure: null,
      status: 'synced',
      values: { language: 'zh-CN', lobbyView: 'scene' },
    });
    expect(gateway.writes).toHaveLength(0);
    unsubscribe();
  });

  it('并发字段修改经合并后收敛，不发生整份文档最后写覆盖', async () => {
    const initial = document('DEVICE_A');
    const concurrent = updated(initial, 'lobbyView', 'list', 'DEVICE_B');
    const gateway = new FakePreferencesGateway();
    gateway.remote = initial;
    gateway.onWrite = (candidate) => mergeAccountPreferencesDocuments(candidate, concurrent);
    const store = new AccountPreferencesStore(gateway, {
      language: 'system',
      lobbyView: 'scene',
    });
    const unsubscribe = store.subscribe(vi.fn());
    await expectSnapshot(store, 'synced');

    expect(store.update('language', 'zh-CN')).toEqual({ ok: true, value: undefined });
    await expectSnapshot(store, 'synced');

    expect(store.getSnapshot().values).toEqual({ language: 'zh-CN', lobbyView: 'list' });
    expect(gateway.writes).toHaveLength(1);
    unsubscribe();
  });

  it.each([
    { label: '设备乙最后写入', writeOrder: [0, 1] as const },
    { label: '设备甲最后写入', writeOrder: [1, 0] as const },
  ])('两个设备真实竞争时最终收敛：$label', async ({ writeOrder }) => {
    const server = new ContendedPreferencesServer(document('INITIAL_DEVICE'));
    const storeA = new AccountPreferencesStore(server.gateway('DEVICE_A'), {
      language: 'system',
      lobbyView: 'scene',
    });
    const storeB = new AccountPreferencesStore(server.gateway('DEVICE_B'), {
      language: 'system',
      lobbyView: 'scene',
    });
    const unsubscribeA = storeA.subscribe(vi.fn());
    const unsubscribeB = storeB.subscribe(vi.fn());
    await Promise.all([expectSnapshot(storeA, 'synced'), expectSnapshot(storeB, 'synced')]);

    server.captureConcurrentWrites();
    expect(storeA.update('language', 'zh-CN')).toEqual({ ok: true, value: undefined });
    expect(storeB.update('lobbyView', 'list')).toEqual({ ok: true, value: undefined });
    await vi.waitFor(() => {
      expect(server.capturedWriteCount()).toBe(2);
    });

    server.releaseConcurrentWrites(writeOrder);

    await vi.waitFor(() => {
      expect(server.values()).toEqual({ language: 'zh-CN', lobbyView: 'list' });
      expect(storeA.getSnapshot()).toMatchObject({
        failure: null,
        status: 'synced',
        values: { language: 'zh-CN', lobbyView: 'list' },
      });
      expect(storeB.getSnapshot()).toMatchObject({
        failure: null,
        status: 'synced',
        values: { language: 'zh-CN', lobbyView: 'list' },
      });
    });

    unsubscribeA();
    unsubscribeB();
  });

  it('断线写入保持待同步，网络活动后自动重试并确认', async () => {
    const gateway = new FakePreferencesGateway();
    gateway.remote = document('DEVICE_A');
    const store = new AccountPreferencesStore(gateway, {
      language: 'system',
      lobbyView: 'scene',
    });
    const unsubscribe = store.subscribe(vi.fn());
    await expectSnapshot(store, 'synced');

    gateway.writeFailure = { code: 'preferences.write_failed', retryable: true };
    store.update('lobbyView', 'list');
    await expectSnapshot(store, 'pending');
    expect(store.getSnapshot()).toMatchObject({
      failure: { code: 'preferences.write_failed', retryable: true },
      values: { lobbyView: 'list' },
    });

    gateway.writeFailure = null;
    gateway.notify();
    await expectSnapshot(store, 'synced');
    expect(gateway.remote).toMatchObject({ fields: { lobbyView: { value: 'list' } } });
    unsubscribe();
  });

  it('账户在读取期间切换时丢弃旧响应，绝不把旧账户偏好写入新账户', async () => {
    const oldResponse =
      deferred<Result<AccountPreferencesDocument | null, AccountPreferencesSyncFailure>>();
    const gateway = new FakePreferencesGateway();
    gateway.nextRead = oldResponse.promise;
    const store = new AccountPreferencesStore(gateway, {
      language: 'system',
      lobbyView: 'scene',
    });
    const unsubscribe = store.subscribe(vi.fn());

    gateway.activeScope = { accountId: '@second:agent-room.test', writerId: 'DEVICE_B' };
    gateway.remote = updated(document('DEVICE_B'), 'language', 'en', 'DEVICE_B');
    gateway.notify();
    oldResponse.resolve(ok(updated(document('DEVICE_A'), 'language', 'zh-CN', 'DEVICE_A')));

    await expectSnapshot(store, 'synced');
    expect(store.getSnapshot().values.language).toBe('en');
    expect(gateway.writes).toHaveLength(0);
    unsubscribe();
  });

  it('未登录时只更新本地引导值，获得账户作用域后优先读取远端', async () => {
    const gateway = new FakePreferencesGateway();
    gateway.activeScope = null;
    const store = new AccountPreferencesStore(gateway, {
      language: 'system',
      lobbyView: 'scene',
    });
    const unsubscribe = store.subscribe(vi.fn());
    await expectSnapshot(store, 'local');

    expect(store.update('language', 'zh-CN')).toEqual({ ok: true, value: undefined });
    expect(store.getSnapshot().values.language).toBe('zh-CN');

    gateway.activeScope = { accountId: '@operator:agent-room.test', writerId: 'DEVICE_A' };
    gateway.remote = updated(document('DEVICE_A'), 'language', 'en', 'DEVICE_A');
    gateway.notify();
    await expectSnapshot(store, 'synced');

    expect(store.getSnapshot().values.language).toBe('en');
    expect(gateway.writes).toHaveLength(0);
    unsubscribe();
  });
});

class FakePreferencesGateway implements AccountPreferencesGateway {
  activeScope: AccountPreferencesScope | null = {
    accountId: '@operator:agent-room.test',
    writerId: 'DEVICE_A',
  };
  readonly listeners = new Set<() => void>();
  nextRead: Promise<
    Result<AccountPreferencesDocument | null, AccountPreferencesSyncFailure>
  > | null = null;
  onWrite: (document: AccountPreferencesDocument) => AccountPreferencesDocument = (value) => value;
  remote: AccountPreferencesDocument | null = null;
  writeFailure: AccountPreferencesSyncFailure | null = null;
  readonly writes: AccountPreferencesDocument[] = [];

  read(): Promise<Result<AccountPreferencesDocument | null, AccountPreferencesSyncFailure>> {
    const nextRead = this.nextRead;
    if (nextRead !== null) {
      this.nextRead = null;
      return nextRead;
    }
    return Promise.resolve(ok(this.remote));
  }

  scope(): AccountPreferencesScope | null {
    return this.activeScope;
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  write(
    documentValue: AccountPreferencesDocument,
  ): Promise<Result<void, AccountPreferencesSyncFailure>> {
    const failure = this.writeFailure;
    if (failure !== null) {
      return Promise.resolve(err(failure));
    }
    this.writes.push(documentValue);
    this.remote = this.onWrite(documentValue);
    return Promise.resolve(ok(undefined));
  }

  notify(): void {
    for (const listener of this.listeners) {
      listener();
    }
  }
}

class ContendedPreferencesServer {
  readonly #listeners = new Set<() => void>();
  #captureWrites = false;
  #capturedWrites: {
    readonly document: AccountPreferencesDocument;
    readonly resolve: (result: Result<void, AccountPreferencesSyncFailure>) => void;
  }[] = [];
  #remote: AccountPreferencesDocument;

  constructor(initial: AccountPreferencesDocument) {
    this.#remote = initial;
  }

  gateway(writerId: string): AccountPreferencesGateway {
    return {
      read: () => Promise.resolve(ok(this.#remote)),
      scope: () => ({ accountId: '@operator:agent-room.test', writerId }),
      subscribe: (listener) => {
        this.#listeners.add(listener);
        return () => {
          this.#listeners.delete(listener);
        };
      },
      write: (documentValue) => this.#write(documentValue),
    };
  }

  captureConcurrentWrites(): void {
    this.#captureWrites = true;
  }

  capturedWriteCount(): number {
    return this.#capturedWrites.length;
  }

  releaseConcurrentWrites(writeOrder: readonly [number, number]): void {
    if (this.#capturedWrites.length !== writeOrder.length) {
      throw new Error('并发写入尚未全部到达测试栅栏。');
    }
    const captured = this.#capturedWrites;
    this.#capturedWrites = [];
    this.#captureWrites = false;
    for (const index of writeOrder) {
      const write = captured[index];
      if (write === undefined) {
        throw new Error('并发写入顺序包含无效索引。');
      }
      this.#remote = write.document;
    }
    for (const write of captured) {
      write.resolve(ok(undefined));
    }
    this.#notify();
  }

  values(): AccountPreferenceValues {
    return valuesFromAccountPreferences(this.#remote);
  }

  #write(
    documentValue: AccountPreferencesDocument,
  ): Promise<Result<void, AccountPreferencesSyncFailure>> {
    if (!this.#captureWrites) {
      this.#remote = documentValue;
      this.#notify();
      return Promise.resolve(ok(undefined));
    }
    return new Promise((resolve) => {
      this.#capturedWrites.push({ document: documentValue, resolve });
    });
  }

  #notify(): void {
    for (const listener of this.#listeners) {
      listener();
    }
  }
}

function document(writerId: string): AccountPreferencesDocument {
  const result = createAccountPreferencesDocument(
    { language: 'system', lobbyView: 'scene' },
    writerId,
  );
  if (!result.ok) {
    throw new Error('测试文档创建失败。');
  }
  return result.value;
}

function updated<TKey extends keyof AccountPreferenceValues>(
  source: AccountPreferencesDocument,
  key: TKey,
  value: AccountPreferenceValues[TKey],
  writerId: string,
): AccountPreferencesDocument {
  const result = updateAccountPreference(source, key, value, writerId);
  if (!result.ok) {
    throw new Error('测试文档更新失败。');
  }
  return result.value;
}

async function expectSnapshot(
  store: AccountPreferencesStore,
  status: ReturnType<AccountPreferencesStore['getSnapshot']>['status'],
): Promise<void> {
  await vi.waitFor(() => {
    expect(store.getSnapshot().status).toBe(status);
  });
}

function deferred<TValue>() {
  let resolvePromise: (value: TValue) => void = () => undefined;
  const promise = new Promise<TValue>((resolve) => {
    resolvePromise = resolve;
  });
  return { promise, resolve: resolvePromise };
}
