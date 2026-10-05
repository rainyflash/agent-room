// @vitest-environment jsdom

import type { ICreateClientOpts } from 'matrix-js-sdk';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { DeviceSigningOutcome, ensureDeviceSigned } from './matrix-device-signing';
import {
  CRYPTO_STORE_CLEAR_WAIT_MS,
  MatrixWebGateway,
  SIGNING_STOP_WAIT_MS,
} from './matrix-web-gateway';
import type { MatrixSessionVault, StoredMatrixSession } from '../domain/matrix-session-vault';
import { DeviceSigningStatus } from '@/shared/matrix/device-signing-status';
import { err, ok } from '@/shared/result';

const sdk = vi.hoisted(() => ({
  options: [] as ICreateClientOpts[],
  whoami: vi.fn<(options: ICreateClientOpts) => Promise<unknown>>(),
  login: vi.fn(),
  loginFlows: vi.fn<() => Promise<{ flows: { type: string }[] }>>(),
  ssoUrl: vi.fn(() => 'https://matrix.test/sso'),
  refresh: vi.fn(),
  logout: vi.fn(),
  stop: vi.fn(),
  abort: vi.fn(),
  clearStores: vi.fn(),
  initializeCrypto:
    vi.fn<(options: { cryptoDatabasePrefix: string; useIndexedDB: boolean }) => Promise<void>>(),
  downloadKeys: vi.fn<(users: string[]) => Promise<unknown>>(),
  ownDeviceKeys: vi.fn<() => Promise<{ ed25519: string; curve25519: string }>>(),
}));

vi.mock('matrix-js-sdk', () => ({
  createClient: (options: ICreateClientOpts) => {
    sdk.options.push(options);
    return {
      loginRequest: sdk.login,
      loginFlows: sdk.loginFlows,
      getSsoLoginUrl: sdk.ssoUrl,
      refreshToken: sdk.refresh,
      logout: sdk.logout,
      whoami: () => sdk.whoami(options),
      initRustCrypto: sdk.initializeCrypto,
      downloadKeysForUsers: sdk.downloadKeys,
      getCrypto: () => ({
        setTrustCrossSignedDevices: vi.fn(),
        setDeviceIsolationMode: vi.fn(),
        getOwnDeviceKeys: sdk.ownDeviceKeys,
      }),
      getDeviceId: () => options.deviceId,
      getUserId: () => options.userId,
      getSyncState: () => 'PREPARED',
      stopClient: sdk.stop,
      http: { abort: sdk.abort },
      clearStores: sdk.clearStores,
      on: vi.fn(),
      removeListener: vi.fn(),
    };
  },
  MemoryStore: class {
    startup = vi.fn();
    deleteAllData = vi.fn();
  },
  IndexedDBStore: class {
    startup = vi.fn();
    deleteAllData = vi.fn();
  },
  ClientEvent: { Sync: 'sync' },
  SyncState: { Prepared: 'PREPARED', Syncing: 'SYNCING' },
  SSOAction: { LOGIN: 'login' },
}));
const signing = vi.hoisted(() => ({
  ensure: vi.fn<typeof ensureDeviceSigned>(),
}));

vi.mock('./matrix-device-signing', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  ensureDeviceSigned: signing.ensure,
}));
vi.mock('matrix-js-sdk/lib/crypto-api/index.js', () => ({
  OnlySignedDevicesIsolationMode: class {
    readonly kind = 'signed-only';
  },
}));

const CLEANUP_KEY = 'agent-room.matrix-crypto-cleanup.v1';

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

/** 本机加密库持久保存的网关，和真的浏览器、桌面端一样：有 IndexedDB，有跨窗口的锁。 */
function persistentGateway(
  vault: MatrixSessionVault,
  persistStorage = vi.fn(() => Promise.resolve(true)),
) {
  const locks = {
    request: (name: string, _options: unknown, granted: (lock: Lock) => Promise<unknown>) =>
      granted({ mode: 'exclusive', name }),
  };
  vi.stubGlobal('navigator', { locks, onLine: true });
  return new MatrixWebGateway({
    baseUrl: 'https://matrix.test',
    indexedDB: {} as IDBFactory,
    persistStorage,
    sessionVault: vault,
    url: () => new URL('https://tauri.localhost/connect'),
  });
}

/** 服务器替账户保管签名钥匙的网关：首次同步之后自动签名。 */
function signingGateway(vault: MatrixSessionVault, deviceSigning: DeviceSigningStatus) {
  return new MatrixWebGateway({
    baseUrl: 'https://matrix.test',
    deviceSigning,
    encryptionKeyEscrow: {
      fetch: vi.fn(() => Promise.resolve(null)),
      replaceCrossSigningKeys: vi.fn(() => Promise.resolve()),
      store: vi.fn(() => Promise.resolve()),
    },
    sessionVault: vault,
  });
}

/** 恢复会话、等首次同步完成，这时自动签名开始跑；返回叫停它用的信号。 */
async function startSigning(matrix: MatrixWebGateway): Promise<AbortSignal> {
  const restored = await matrix.restore(session.userId);
  if (!restored.ok || restored.value.kind !== 'connected') throw new Error('测试必须先连上');
  await expect(restored.value.connection.waitUntilPrepared()).resolves.toEqual(ok(undefined));
  const stop = signing.ensure.mock.calls[0]?.[3];
  if (stop === undefined) throw new Error('自动签名必须带着叫停用的信号');
  return stop;
}

/** 服务器上记着的这台设备的签名公钥。 */
function recordedKeys(ed25519: string) {
  return {
    device_keys: {
      [session.userId]: {
        [session.deviceId]: { keys: { [`ed25519:${session.deviceId}`]: ed25519 } },
      },
    },
    failures: {},
  };
}

describe('Matrix 网关持久会话生命周期', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sdk.options.length = 0;
    sdk.logout.mockReset().mockResolvedValue(undefined);
    sdk.clearStores.mockReset().mockResolvedValue(undefined);
    sdk.initializeCrypto.mockReset().mockResolvedValue(undefined);
    sdk.downloadKeys.mockReset().mockResolvedValue({ device_keys: {}, failures: {} });
    sdk.ownDeviceKeys.mockReset().mockResolvedValue({ ed25519: 'local-key', curve25519: 'local' });
    sdk.loginFlows.mockReset().mockResolvedValue({ flows: [{ type: 'm.login.sso' }] });
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

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('从连接页重新授权仍保留最初房间深链', async () => {
    sessionStorage.setItem('agent-room.matrix-return-path.v1', '/lobby/public?directory=open');
    const navigate = vi.fn();
    const matrix = new MatrixWebGateway({
      baseUrl: 'https://matrix.test',
      sessionVault: storage(null),
      navigate,
      url: () => new URL('https://app.test/connect'),
    });
    await expect(matrix.beginAuthentication('/connect')).resolves.toEqual(
      ok({ kind: 'browser-navigation' }),
    );
    expect(sessionStorage.getItem('agent-room.matrix-return-path.v1')).toBe(
      '/lobby/public?directory=open',
    );
    expect(navigate).toHaveBeenCalledExactlyOnceWith('https://matrix.test/sso');
  });

  it('退出后迟到的登录方式响应不能再次跳转授权', async () => {
    const response = Promise.withResolvers<{ flows: { type: string }[] }>();
    sdk.loginFlows.mockReturnValueOnce(response.promise);
    const navigate = vi.fn();
    const matrix = new MatrixWebGateway({
      baseUrl: 'https://matrix.test',
      sessionVault: storage(null),
      navigate,
    });
    const authentication = matrix.beginAuthentication('/rooms');
    await vi.waitFor(() => {
      expect(sdk.loginFlows).toHaveBeenCalledOnce();
    });
    await matrix.logout();
    response.resolve({ flows: [{ type: 'm.login.sso' }] });
    await expect(authentication).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.session_superseded' },
    });
    expect(navigate).not.toHaveBeenCalled();
    expect(sessionStorage.getItem('agent-room.matrix-return-path.v1')).toBeNull();
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
    const loggingOut = matrix.logout();
    expect(sdk.logout).not.toHaveBeenCalled();
    response.resolve({ user_id: session.userId, device_id: session.deviceId });
    await expect(loggingOut).resolves.toEqual(ok(undefined));
    await expect(restoring).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.session_superseded' },
    });
    expect(sdk.initializeCrypto).not.toHaveBeenCalled();
    await expect(vault.load()).resolves.toEqual(ok(null));
  });

  it('退出等待加密初始化结束并取消后台请求后才撤销令牌和清理当前设备', async () => {
    const initialization = Promise.withResolvers<undefined>();
    sdk.initializeCrypto.mockReturnValueOnce(initialization.promise);
    const vault = storage();
    const onClientChange = vi.fn();
    const matrix = new MatrixWebGateway({
      baseUrl: 'https://matrix.test',
      sessionVault: vault,
      onClientChange,
    });
    const restoring = matrix.restore(session.userId);
    await vi.waitFor(() => {
      expect(sdk.initializeCrypto).toHaveBeenCalledOnce();
    });
    const loggingOut = matrix.logout();
    await expect(matrix.restore(session.userId)).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.logout_incomplete' },
    });
    expect(sdk.logout).not.toHaveBeenCalled();
    expect(sdk.clearStores).not.toHaveBeenCalled();
    initialization.resolve(undefined);
    await expect(restoring).resolves.toMatchObject({
      ok: false,
      error: { code: 'matrix.session_superseded' },
    });
    await expect(loggingOut).resolves.toEqual(ok(undefined));
    expect(onClientChange.mock.calls.every(([client]) => client === null)).toBe(true);
    expect(sdk.abort).toHaveBeenCalled();
    const revokedAt = sdk.logout.mock.invocationCallOrder[0];
    if (revokedAt === undefined) throw new Error('退出必须撤销远端会话');
    expect(sdk.abort.mock.invocationCallOrder[0]).toBeLessThan(revokedAt);
    expect(sdk.clearStores).toHaveBeenCalledExactlyOnceWith({
      cryptoDatabasePrefix: sdk.initializeCrypto.mock.calls[0]?.[0].cryptoDatabasePrefix,
    });
    await expect(vault.load()).resolves.toEqual(ok(null));
  });

  it('退出取消仍在加载的推送规则时保留诊断，不误报同步故障', async () => {
    const errorLog = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const debugLog = vi.spyOn(console, 'debug').mockImplementation(() => undefined);
    try {
      const vault = storage();
      const matrix = gateway(vault);
      const restored = await matrix.restore(session.userId);
      expect(restored).toMatchObject({ ok: true, value: { kind: 'connected' } });
      const logger = sdk.options.find((options) => options.deviceId === session.deviceId)?.logger;
      if (logger === undefined) throw new Error('同步客户端必须拥有独立的生命周期日志');
      const child = logger.getChild('sync');
      const cancellation = new DOMException('signal is aborted without reason', 'AbortError');
      sdk.abort.mockImplementationOnce(() => {
        child.error('Getting push rules failed', cancellation);
      });
      await expect(matrix.logout()).resolves.toEqual(ok(undefined));
      expect(sdk.abort).toHaveBeenCalled();
      expect(errorLog).not.toHaveBeenCalled();
      expect(debugLog).toHaveBeenCalledWith('sync', 'Getting push rules failed', cancellation);
      await expect(vault.load()).resolves.toEqual(ok(null));
    } finally {
      errorLog.mockRestore();
      debugLog.mockRestore();
    }
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

  it('本机加密库删除被挡住时退出不再一直等，删除完成前记着留到下次启动', async () => {
    const matrix = gateway(storage());
    await matrix.restore(session.userId);
    const initialized = sdk.initializeCrypto.mock.calls[0]?.[0];
    if (initialized === undefined) throw new Error('测试必须先初始化当前设备的加密库');
    let finishClearing: () => void = () => undefined;
    sdk.clearStores.mockReturnValueOnce(
      new Promise<void>((resolve) => {
        finishClearing = resolve;
      }),
    );

    vi.useFakeTimers();
    try {
      const logout = matrix.logout();
      await vi.advanceTimersByTimeAsync(CRYPTO_STORE_CLEAR_WAIT_MS);
      await expect(logout).resolves.toEqual(ok(undefined));
    } finally {
      vi.useRealTimers();
    }
    expect(JSON.parse(localStorage.getItem(CLEANUP_KEY) ?? '[]')).toEqual([
      initialized.cryptoDatabasePrefix,
    ]);

    // 加密模块放手后删除自己完成，就不用再记着；退出也不会再清理一遍。
    finishClearing();
    await vi.waitFor(() => {
      expect(localStorage.getItem(CLEANUP_KEY)).toBeNull();
    });
    await expect(matrix.logout()).resolves.toEqual(ok(undefined));
    expect(sdk.clearStores).toHaveBeenCalledOnce();
    expect(sdk.logout).toHaveBeenCalledOnce();
  });

  it('重试收到已撤销的 401 时继续清理本地存储', async () => {
    const matrix = gateway(storage());
    await matrix.restore(session.userId);
    sdk.logout.mockRejectedValueOnce({ httpStatus: 401 });

    await expect(matrix.logout()).resolves.toEqual(ok(undefined));
    expect(sdk.clearStores).toHaveBeenCalledOnce();
  });

  it('本机加密存储丢过、同一个设备号换了密钥时注销这台设备，清掉本机会话，要求重新登录', async () => {
    sdk.downloadKeys.mockResolvedValue(recordedKeys('recorded-key'));
    sdk.ownDeviceKeys.mockResolvedValue({ ed25519: 'replaced-key', curve25519: 'local' });
    const vault = storage();
    const persistStorage = vi.fn(() => Promise.resolve(true));

    await expect(persistentGateway(vault, persistStorage).restore(session.userId)).resolves.toEqual(
      ok({ kind: 'authentication-required' }),
    );

    // 服务器上的是加密库起来以前问的：起来以后它可能已经把新密钥传上去了。
    const queried = sdk.downloadKeys.mock.invocationCallOrder[0];
    const initialized = sdk.initializeCrypto.mock.invocationCallOrder[0];
    if (queried === undefined || initialized === undefined) throw new Error('两步都要做');
    expect(queried).toBeLessThan(initialized);
    expect(sdk.initializeCrypto.mock.calls[0]?.[0].useIndexedDB).toBe(true);
    expect(sdk.logout).toHaveBeenCalledExactlyOnceWith(true);
    expect(sdk.clearStores).toHaveBeenCalledExactlyOnceWith({
      cryptoDatabasePrefix: sdk.initializeCrypto.mock.calls[0]?.[0].cryptoDatabasePrefix,
    });
    await expect(vault.load()).resolves.toEqual(ok(null));
    expect(persistStorage).not.toHaveBeenCalled();
  });

  it('密钥对得上时照常连上，并请浏览器持久保存本站数据（只请求一次）', async () => {
    sdk.downloadKeys.mockResolvedValue(recordedKeys('local-key'));
    const persistStorage = vi.fn(() => Promise.resolve(true));
    const matrix = persistentGateway(storage(), persistStorage);

    for (let round = 0; round < 2; round += 1) {
      await expect(matrix.restore(session.userId)).resolves.toMatchObject({
        ok: true,
        value: { kind: 'connected' },
      });
    }
    expect(sdk.logout).not.toHaveBeenCalled();
    expect(persistStorage).toHaveBeenCalledOnce();
  });

  it('新设备服务器上还没有密钥，或者问不到时照常连上', async () => {
    sdk.downloadKeys.mockRejectedValueOnce(new Error('网络中断'));
    const matrix = persistentGateway(storage());
    for (let round = 0; round < 2; round += 1) {
      await expect(matrix.restore(session.userId)).resolves.toMatchObject({
        ok: true,
        value: { kind: 'connected' },
      });
    }
    expect(sdk.downloadKeys).toHaveBeenCalledTimes(2);
    expect(sdk.logout).not.toHaveBeenCalled();
  });

  it('本机加密库只放在内存里时不比密钥：每次起来都是一套新的', async () => {
    sdk.downloadKeys.mockResolvedValue(recordedKeys('recorded-key'));
    sdk.ownDeviceKeys.mockResolvedValue({ ed25519: 'replaced-key', curve25519: 'local' });
    await expect(gateway(storage()).restore(session.userId)).resolves.toMatchObject({
      ok: true,
      value: { kind: 'connected' },
    });
    expect(sdk.downloadKeys).not.toHaveBeenCalled();
    expect(sdk.logout).not.toHaveBeenCalled();
  });

  it('退出时先叫停还在跑的自动签名，等它停下才撤销令牌；叫停不算签名失败', async () => {
    // 令牌作废以后签名流程再读账户数据只会拿到 401（2026-10-05 真实登录验收抓到的）。
    const running = Promise.withResolvers<DeviceSigningOutcome>();
    signing.ensure.mockReturnValueOnce(running.promise);
    const vault = storage();
    const status = new DeviceSigningStatus();
    const matrix = signingGateway(vault, status);
    const stop = await startSigning(matrix);
    expect(stop.aborted).toBe(false);

    const loggingOut = matrix.logout();
    await vi.waitFor(() => {
      expect(vault.clear).toHaveBeenCalledOnce();
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(stop.aborted).toBe(true);
    expect(sdk.logout).not.toHaveBeenCalled();

    running.reject(stop.reason);
    await expect(loggingOut).resolves.toEqual(ok(undefined));
    expect(sdk.logout).toHaveBeenCalledOnce();
    expect(status.getSnapshot().kind).not.toBe('failed');
    // 退出以后界面上的“重试”也不会再跑一遍。
    status.retry();
    expect(signing.ensure).toHaveBeenCalledOnce();
  });

  it('自动签名一直停不下来时，退出最多等一会就照常撤销令牌', async () => {
    signing.ensure.mockReturnValueOnce(new Promise(() => undefined));
    const matrix = signingGateway(storage(), new DeviceSigningStatus());
    await startSigning(matrix);

    vi.useFakeTimers();
    try {
      const logout = matrix.logout();
      await vi.advanceTimersByTimeAsync(SIGNING_STOP_WAIT_MS);
      await expect(logout).resolves.toEqual(ok(undefined));
    } finally {
      vi.useRealTimers();
    }
    expect(sdk.logout).toHaveBeenCalledOnce();
  });
});
