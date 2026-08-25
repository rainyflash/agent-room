// @vitest-environment jsdom

import { describe, expect, it } from 'vitest';

import type { MatrixClient } from 'matrix-js-sdk';
import type { CryptoApi, DeviceIsolationMode } from 'matrix-js-sdk/lib/crypto-api/index.js';

import { initializeMatrixCrypto, MatrixWebGateway } from './matrix-web-gateway';

describe('MatrixWebGateway', () => {
  it('没有当前标签页凭据时明确请求 SSO', async () => {
    const gateway = new MatrixWebGateway({
      baseUrl: 'https://matrix.agent-room.test',
      sessionStorage: memoryStorage(),
      url: () => new URL('https://app.agent-room.test/connect'),
    });

    await expect(gateway.restore('@user:matrix.agent-room.test')).resolves.toEqual({
      ok: true,
      value: { kind: 'authentication-required' },
    });
  });

  it('拒绝 connect 之外的 loginToken 并立即从 URL 清除', async () => {
    const replacements: string[] = [];
    const gateway = new MatrixWebGateway({
      baseUrl: 'https://matrix.agent-room.test',
      replaceHistory: (url) => {
        replacements.push(url);
      },
      sessionStorage: memoryStorage(),
      url: () => new URL('https://app.agent-room.test/lobby/public?loginToken=single-use'),
    });

    const result = await gateway.restore('@user:matrix.agent-room.test');

    expect(replacements).toEqual(['/lobby/public']);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error.code).toBe('matrix.invalid_sso_callback_path');
    }
  });

  it('登出即使没有活跃客户端也会清除当前标签页状态', async () => {
    const storage = memoryStorage({
      'agent-room.matrix-return-path.v1': '/lobby/public',
      'agent-room.matrix-session.v1': '{"credential":"secret"}',
    });
    const gateway = new MatrixWebGateway({
      baseUrl: 'https://matrix.agent-room.test',
      sessionStorage: storage,
      url: () => new URL('https://app.agent-room.test/connect'),
    });

    await expect(gateway.logout()).resolves.toEqual({ ok: true, value: undefined });
    expect(storage.length).toBe(0);
  });

  it('恢复和登出边界都会先撤下旧客户端租约', async () => {
    const observedClients: null[] = [];
    const gateway = new MatrixWebGateway({
      baseUrl: 'https://matrix.agent-room.test',
      onClientChange: (client) => {
        if (client === null) {
          observedClients.push(client);
        }
      },
      sessionStorage: memoryStorage(),
      url: () => new URL('https://app.agent-room.test/connect'),
    });

    await gateway.restore('@user:matrix.agent-room.test');
    await gateway.logout();

    expect(observedClients).toEqual([null, null]);
  });

  it('使用独立持久库初始化 Rust Crypto 并只信任已签名设备', async () => {
    const calls: unknown[] = [];
    const isolationMode = { kind: 'signed-only' } as unknown as DeviceIsolationMode;
    const crypto = {
      setDeviceIsolationMode: (value: DeviceIsolationMode) => {
        calls.push(['isolation', value]);
      },
      setTrustCrossSignedDevices: (value: boolean) => {
        calls.push(['cross-signing', value]);
      },
    } as Pick<CryptoApi, 'setDeviceIsolationMode' | 'setTrustCrossSignedDevices'>;
    const client = {
      getCrypto: () => crypto,
      initRustCrypto: (options: unknown) => {
        calls.push(['initialize', options]);
        return Promise.resolve();
      },
    } as unknown as Pick<MatrixClient, 'getCrypto' | 'initRustCrypto'>;

    await initializeMatrixCrypto(client, {
      databasePrefix: 'agent-room-crypto-device',
      isolationMode,
      persistent: true,
    });

    expect(calls).toEqual([
      ['initialize', { cryptoDatabasePrefix: 'agent-room-crypto-device', useIndexedDB: true }],
      ['cross-signing', true],
      ['isolation', isolationMode],
    ]);
  });

  it('Rust Crypto 初始化后不可用时失败关闭', async () => {
    const client = {
      getCrypto: () => undefined,
      initRustCrypto: () => Promise.resolve(),
    } as unknown as Pick<MatrixClient, 'getCrypto' | 'initRustCrypto'>;

    await expect(
      initializeMatrixCrypto(client, {
        databasePrefix: 'agent-room-crypto-device',
        isolationMode: { kind: 'signed-only' } as unknown as DeviceIsolationMode,
        persistent: true,
      }),
    ).rejects.toThrow('Matrix Rust Crypto 初始化完成后仍不可用。');
  });
});

function memoryStorage(initial: Readonly<Record<string, string>> = {}): Storage {
  const values = new Map(Object.entries(initial));
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
