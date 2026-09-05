// @vitest-environment jsdom

import type { ICreateClientOpts } from 'matrix-js-sdk';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { MatrixWebGateway } from './matrix-web-gateway';
import type { MatrixSessionVault, StoredMatrixSession } from '../domain/matrix-session-vault';
import { err, ok } from '@/shared/result';

const sdk = vi.hoisted(() => ({
  options: [] as ICreateClientOpts[],
  whoami: vi.fn<(options: ICreateClientOpts) => Promise<unknown>>(),
  login: vi.fn(),
  refresh: vi.fn(),
  logout: vi.fn(),
  stop: vi.fn(),
  clearStores: vi.fn(),
  initializeCrypto:
    vi.fn<(options: { cryptoDatabasePrefix: string; useIndexedDB: boolean }) => Promise<void>>(),
}));

vi.mock('matrix-js-sdk', () => ({
  createClient: (options: ICreateClientOpts) => {
    sdk.options.push(options);
    return {
      loginRequest: sdk.login,
      refreshToken: sdk.refresh,
      logout: sdk.logout,
      whoami: () => sdk.whoami(options),
      initRustCrypto: sdk.initializeCrypto,
      getCrypto: () => ({ setTrustCrossSignedDevices: vi.fn(), setDeviceIsolationMode: vi.fn() }),
      getDeviceId: () => options.deviceId,
      getUserId: () => options.userId,
      getSyncState: () => 'PREPARED',
      stopClient: sdk.stop,
      clearStores: sdk.clearStores,
      on: vi.fn(),
      removeListener: vi.fn(),
    };
  },
  MemoryStore: class {
    startup = vi.fn();
    deleteAllData = vi.fn();
  },
  ClientEvent: { Sync: 'sync' },
  SyncState: { Prepared: 'PREPARED', Syncing: 'SYNCING' },
}));
vi.mock('matrix-js-sdk/lib/crypto-api/index.js', () => ({
  OnlySignedDevicesIsolationMode: class {
    readonly kind = 'signed-only';
  },
}));

const session: StoredMatrixSession = {
  accessToken: 'test-access',
  deviceId: 'UNCHANGED_DEVICE',
  refreshToken: 'test-refresh',
  userId: '@tester:matrix.test',
  version: 1,
};

function storage(initial: StoredMatrixSession | null = session) {
  let stored = initial;
  return {
    load: vi.fn<MatrixSessionVault['load']>(() => Promise.resolve(ok(stored))),
    save: vi.fn<MatrixSessionVault['save']>((value) => {
      stored = value;
      return Promise.resolve(ok(undefined));
    }),
    clear: vi.fn<MatrixSessionVault['clear']>(() => {
      stored = null;
      return Promise.resolve(ok(undefined));
    }),
  };
}

function gateway(vault: MatrixSessionVault) {
  return new MatrixWebGateway({
    baseUrl: 'https://matrix.test',
    sessionVault: vault,
    url: () => new URL('https://tauri.localhost/connect'),
  });
}

describe('Matrix 网关持久会话生命周期', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sdk.options.length = 0;
    sdk.logout.mockReset().mockResolvedValue(undefined);
    sdk.clearStores.mockReset().mockResolvedValue(undefined);
    sessionStorage.clear();
    localStorage.clear();
    sdk.whoami.mockImplementation((options) =>
      Promise.resolve({ user_id: options.userId, device_id: options.deviceId }),
    );
    sdk.login.mockResolvedValue({
      access_token: session.accessToken,
      device_id: session.deviceId,
      refresh_token: session.refreshToken,
      user_id: session.userId,
    });
    sdk.refresh.mockResolvedValue({
      access_token: 'rotated-access',
      refresh_token: 'rotated-refresh',
      expires_in_ms: 60_000,
    });
  });

  it('首次认证持久化后新建网关无需再 SSO 且复用同一设备的加密库', async () => {
    const vault = storage(null);
    await gateway(vault).exchangeAuthenticationGrant('single-use', '/rooms');
    const restored = await gateway(vault).restore(session.userId);
    expect(restored).toMatchObject({
      ok: true,
      value: {
        kind: 'connected',
        connection: { deviceId: session.deviceId, userId: session.userId },
      },
    });
    expect(sdk.login).toHaveBeenCalledTimes(1);
    const firstCryptoOptions: unknown = sdk.initializeCrypto.mock.calls[0]?.[0];
    await gateway(vault).restore(session.userId);
    expect(sdk.initializeCrypto.mock.calls[1]?.[0]).toEqual(firstCryptoOptions);
    expect(sessionStorage.getItem('agent-room.matrix-session.v1')).toBeNull();
  });

  it('账户切换清除旧凭据而不冒充新账户', async () => {
    const vault = storage();
    await expect(gateway(vault).restore('@another:matrix.test')).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.identity_mismatch' },
    });
    expect(vault.clear).toHaveBeenCalledOnce();
    expect(sdk.initializeCrypto).not.toHaveBeenCalled();
  });

  it('刷新响应省略可选字段时继续保留原刷新令牌', async () => {
    sdk.refresh.mockResolvedValueOnce({ access_token: 'new-access' });
    const vault = storage();
    await gateway(vault).restore(session.userId);
    const options = sdk.options.at(-1);
    await expect(options?.tokenRefreshFunction?.('test-refresh')).resolves.toEqual({
      accessToken: 'new-access',
      refreshToken: 'test-refresh',
    });
    await expect(vault.load()).resolves.toEqual(ok({ ...session, accessToken: 'new-access' }));
  });

  it('服务端撤销会话后清理持久凭据并请求重新认证', async () => {
    sdk.whoami.mockRejectedValueOnce({ httpStatus: 401 });
    const vault = storage();
    await expect(gateway(vault).restore(session.userId)).resolves.toEqual(
      ok({ kind: 'authentication-required' }),
    );
    await expect(vault.load()).resolves.toEqual(ok(null));
    expect(sdk.stop).toHaveBeenCalled();
  });

  it('SDK 把刷新写入错误包装成 401 时仍保留真实存储故障', async () => {
    sdk.whoami.mockImplementationOnce(async (options) => {
      try {
        await options.tokenRefreshFunction?.('test-refresh');
      } catch {
        throw Object.assign(new Error('SDK 包装后的刷新故障'), { httpStatus: 401 });
      }
      return { user_id: session.userId, device_id: session.deviceId };
    });
    const vault = storage();
    vault.save.mockResolvedValueOnce(
      err({
        boundary: 'matrix',
        code: 'desktop.matrix_session.vault_unavailable',
        offline: false,
        retryable: true,
      }),
    );
    const matrix = gateway(vault);
    await expect(matrix.restore(session.userId)).resolves.toMatchObject({
      ok: false,
      error: { code: 'desktop.matrix_session.vault_unavailable' },
    });
    expect(vault.clear).not.toHaveBeenCalled();
    await expect(matrix.restore(session.userId)).resolves.toMatchObject({
      ok: true,
      value: { kind: 'connected' },
    });
    expect(sdk.options.at(-1)?.accessToken).toBe('rotated-access');
  });

  it('退出与仍在恢复的 whoami 并发时不能重新发布连接', async () => {
    const response = Promise.withResolvers<unknown>();
    const started = Promise.withResolvers<undefined>();
    sdk.whoami.mockImplementationOnce(() => {
      started.resolve(undefined);
      return response.promise;
    });
    const vault = storage();
    const matrix = gateway(vault);
    const restoring = matrix.restore(session.userId);
    await started.promise;
    await matrix.logout();
    response.resolve({ user_id: session.userId, device_id: session.deviceId });
    await expect(restoring).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.session_superseded' },
    });
    expect(sdk.initializeCrypto).not.toHaveBeenCalled();
    await expect(vault.load()).resolves.toEqual(ok(null));
  });

  it('退出清理失败必须上报但仍尝试关闭远端会话', async () => {
    const vault = storage();
    const matrix = gateway(vault);
    await matrix.restore(session.userId);
    vault.clear.mockResolvedValueOnce(
      err({
        boundary: 'matrix',
        code: 'desktop.matrix_session.vault_unavailable',
        offline: false,
        retryable: true,
      }),
    );
    await expect(matrix.logout()).resolves.toMatchObject({
      ok: false,
      error: { code: 'desktop.matrix_session.vault_unavailable' },
    });
    expect(sdk.logout).toHaveBeenCalledOnce();
  });

  it('远端退出失败后保留撤销能力且禁止恢复旧会话', async () => {
    const vault = storage();
    const matrix = gateway(vault);
    await matrix.restore(session.userId);
    sdk.logout.mockRejectedValueOnce(new Error('网络中断'));

    await expect(matrix.logout()).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.logout_failed' },
    });
    await expect(vault.load()).resolves.toEqual(ok(null));
    await expect(matrix.restore(session.userId)).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.logout_incomplete' },
    });
    await expect(matrix.logout()).resolves.toEqual(ok(undefined));
    expect(sdk.logout).toHaveBeenCalledTimes(2);
    await expect(matrix.restore(session.userId)).resolves.toEqual(
      ok({ kind: 'authentication-required' }),
    );
  });

  it('尚未建立连接的持久会话也会被撤销且网络失败后可以重试', async () => {
    const vault = storage();
    const matrix = gateway(vault);
    sdk.logout.mockRejectedValueOnce(new Error('网络中断'));

    await expect(matrix.logout()).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.logout_failed' },
    });
    await expect(vault.load()).resolves.toEqual(ok(null));
    await expect(matrix.restore(session.userId)).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.logout_incomplete' },
    });
    await expect(matrix.logout()).resolves.toEqual(ok(undefined));
    expect(sdk.logout).toHaveBeenCalledTimes(2);
    expect(sdk.options.map((options) => options.accessToken)).toEqual([
      session.accessToken,
      session.accessToken,
    ]);
  });

  it('本地加密库清理失败后重试且不重复已成功的远端撤销', async () => {
    const matrix = gateway(storage());
    await matrix.restore(session.userId);
    sdk.clearStores.mockRejectedValueOnce(new Error('存储暂时不可用'));

    await expect(matrix.logout()).resolves.toMatchObject({ ok: false });
    await expect(matrix.logout()).resolves.toEqual(ok(undefined));
    expect(sdk.logout).toHaveBeenCalledOnce();
    expect(sdk.clearStores).toHaveBeenCalledTimes(2);
    const initialized = sdk.initializeCrypto.mock.calls[0]?.[0];
    if (initialized === undefined) throw new Error('测试必须先初始化当前设备的加密库');
    expect(sdk.clearStores).toHaveBeenLastCalledWith({
      cryptoDatabasePrefix: initialized.cryptoDatabasePrefix,
    });
  });

  it('重试收到已撤销的 401 时继续清理本地存储', async () => {
    const matrix = gateway(storage());
    await matrix.restore(session.userId);
    sdk.logout.mockRejectedValueOnce({ httpStatus: 401 });

    await expect(matrix.logout()).resolves.toEqual(ok(undefined));
    expect(sdk.clearStores).toHaveBeenCalledOnce();
  });
});
