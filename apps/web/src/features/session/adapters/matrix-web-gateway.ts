import type { ClientEvent, MatrixClient, SyncState } from 'matrix-js-sdk';
import type { DeviceIsolationMode } from 'matrix-js-sdk/lib/crypto-api/index.js';
import { z } from 'zod';

import { failure } from '@/features/session/adapters/control-plane-client';
import { IndexedDbMatrixSessionVault } from './indexed-db-matrix-session-vault';
import { acquireMatrixCryptoLease, type MatrixCryptoLease } from './browser-matrix-lease';
import { MatrixCryptoStoreCleanup } from './matrix-crypto-store-cleanup';
import { routeCrossSigningUpload } from './cross-signing-upload-route';
import { type EncryptionKeyEscrow, ensureDeviceSigned } from './matrix-device-signing';
import { ensureFirstEncryptionIdentity } from './matrix-encryption-identity';
import { MatrixLifecycleLogger } from './matrix-lifecycle-logger';
import {
  storedMatrixSessionSchema,
  type MatrixSessionVault,
  type StoredMatrixSession,
} from '@/features/session/domain/matrix-session-vault';
import {
  MatrixSessionRepository,
  supersededMatrixSession,
} from '@/features/session/domain/matrix-session-repository';
import type {
  AuthenticationStartOutcome,
  MatrixConnection,
  MatrixConnectionStatus,
  MatrixGateway,
  MatrixRestoreOutcome,
  SessionFailure,
} from '@/features/session/domain/session';
import { DeviceSigningStatus } from '@/shared/matrix/device-signing-status';
import { MatrixSecretStorageKeyCache } from '@/shared/matrix/matrix-secret-storage-key-cache';
import { err, ok, type Result } from '@/shared/result';

const MATRIX_RETURN_PATH_KEY = 'agent-room.matrix-return-path.v1';
/**
 * 退出时删除本机加密库最多等这么久。Rust 加密模块偶尔要等垃圾回收才关掉数据库连接，
 * 删除会一直被挡住；过了这个时间就先完成退出，删除请求留给浏览器自己完成（见 {@link MatrixCryptoStoreCleanup}）。
 */
export const CRYPTO_STORE_CLEAR_WAIT_MS = 5_000;
/** 退出登录时等自动签名做完手头这一步，最多等这么久。 */
export const SIGNING_STOP_WAIT_MS = 5_000;
/**
 * 同一个页面里接连重连（比如睡眠醒来、网络断了又连上）时，上一次恢复可能还拿着加密库的锁。新的这次先叫停它，
 * 等它收尾放开锁，最多等这么久；它卡在本机存储或加密模块里一直不结束，就报“卡住了”，不当成另一个窗口占着。
 */
export const RESTORE_HANDOFF_WAIT_MS = 30_000;
const MATRIX_SAS_VERIFICATION_METHOD = 'm.sas.v1';
const MAX_LOGIN_TOKEN_LENGTH = 4_096;

const loginResponseSchema = z.looseObject({
  access_token: z.string().min(1),
  device_id: z.string().min(1).max(255),
  refresh_token: z.string().min(1).optional(),
  user_id: z.string().regex(/^@[^:]+:.+$/u),
});

const whoAmISchema = z.looseObject({
  device_id: z.string().optional(),
  user_id: z.string().regex(/^@[^:]+:.+$/u),
});

const keysQuerySchema = z.looseObject({
  device_keys: z
    .record(
      z.string(),
      z.record(z.string(), z.looseObject({ keys: z.record(z.string(), z.string()) })),
    )
    .optional(),
});

// Matrix 规范允许省略刷新令牌和有效期，SDK 42 的返回类型却把它们标成必填。
type MatrixRefreshResponse = {
  readonly access_token: string;
  readonly expires_in_ms?: number;
  readonly refresh_token?: string;
};

export type MatrixWebGatewayOptions = {
  readonly baseUrl: string;
  readonly deviceDisplayName?: string;
  readonly indexedDB?: IDBFactory;
  readonly localStorage?: Storage;
  readonly navigate?: (url: string) => void;
  readonly onClientActivity?: (client: MatrixClient) => void;
  readonly onClientChange?: (client: MatrixClient | null) => void;
  readonly online?: () => boolean;
  /** 请浏览器别在空间紧张时清掉本站数据；默认用 `navigator.storage.persist()`。 */
  readonly persistStorage?: () => Promise<boolean>;
  readonly replaceHistory?: (url: string) => void;
  readonly secretStorageKeys?: MatrixSecretStorageKeyCache;
  /**
   * 服务器替账户保管的签名钥匙（ADR 0011）。给了就在登录后自动签好这台设备；
   * 没给（测试、旧的组装）时只像以前一样给第一台设备建立身份。
   */
  readonly encryptionKeyEscrow?: EncryptionKeyEscrow;
  readonly deviceSigning?: DeviceSigningStatus;
  readonly sessionStorage?: Storage;
  readonly sessionVault?: MatrixSessionVault;
  readonly syncTimeoutMs?: number;
  readonly url?: () => URL;
};

export class MatrixWebGateway implements MatrixGateway {
  readonly #baseUrl: string;
  readonly #deviceDisplayName: string;
  readonly #indexedDB: IDBFactory | undefined;
  readonly #localStorage: Storage | undefined;
  readonly #navigate: (url: string) => void;
  readonly #onClientActivity: (client: MatrixClient) => void;
  readonly #onClientChange: (client: MatrixClient | null) => void;
  readonly #online: () => boolean;
  readonly #persistStorage: () => Promise<boolean>;
  #storagePersistenceRequested = false;
  readonly #replaceHistory: (url: string) => void;
  readonly #secretStorageKeys: MatrixSecretStorageKeyCache;
  readonly #encryptionKeyEscrow: EncryptionKeyEscrow | undefined;
  readonly #deviceSigning: DeviceSigningStatus;
  readonly #sessionStorage: Storage;
  readonly #sessions: MatrixSessionRepository;
  readonly #clientLogs = new WeakMap<MatrixClient, MatrixLifecycleLogger>();
  readonly #cryptoCleanup: MatrixCryptoStoreCleanup;
  #restoreAttempt = 0;
  readonly #syncTimeoutMs: number;
  readonly #url: () => URL;
  #activeConnection: BrowserMatrixConnection | null = null;
  #cryptoLease: MatrixCryptoLease | null = null;
  #pendingLogout: BrowserMatrixConnection | null = null;
  #pendingRevocation: StoredMatrixSession | null = null;
  readonly #pendingRestores = new Set<Promise<Result<MatrixRestoreOutcome, SessionFailure>>>();
  /** 还在恢复中、没交出去的客户端；被取代时叫停它们，好让那次恢复尽快收尾、放开加密库的锁。 */
  readonly #restoringClients = new Set<MatrixClient>();
  #logoutInProgress = false;
  #freshAuthenticationReturnPath: string | undefined;

  constructor({
    baseUrl,
    deviceDisplayName = 'Agent Room Web',
    indexedDB = window.indexedDB,
    localStorage = window.localStorage,
    navigate = (url) => {
      window.location.assign(url);
    },
    onClientActivity = ignoreClientActivity,
    onClientChange = ignoreClientChange,
    online = () => window.navigator.onLine,
    persistStorage = requestPersistentStorage,
    replaceHistory = (url) => {
      window.history.replaceState(window.history.state, '', url);
    },
    secretStorageKeys = new MatrixSecretStorageKeyCache(),
    encryptionKeyEscrow,
    deviceSigning = new DeviceSigningStatus(),
    sessionStorage = window.sessionStorage,
    sessionVault = new IndexedDbMatrixSessionVault(baseUrl, indexedDB, sessionStorage),
    syncTimeoutMs = 20_000,
    url = () => new URL(window.location.href),
  }: MatrixWebGatewayOptions) {
    this.#baseUrl = baseUrl;
    this.#deviceDisplayName = deviceDisplayName;
    this.#indexedDB = indexedDB;
    this.#localStorage = localStorage;
    this.#navigate = navigate;
    this.#onClientActivity = onClientActivity;
    this.#onClientChange = onClientChange;
    this.#online = online;
    this.#persistStorage = persistStorage;
    this.#replaceHistory = replaceHistory;
    this.#secretStorageKeys = secretStorageKeys;
    this.#encryptionKeyEscrow = encryptionKeyEscrow;
    this.#deviceSigning = deviceSigning;
    this.#sessionStorage = sessionStorage;
    this.#sessions = new MatrixSessionRepository(sessionVault);
    this.#syncTimeoutMs = syncTimeoutMs;
    this.#url = url;
    this.#cryptoCleanup = new MatrixCryptoStoreCleanup(localStorage, indexedDB);
    this.#cryptoCleanup.retry();
  }

  async beginAuthentication(
    returnPath: string,
  ): Promise<Result<AuthenticationStartOutcome, SessionFailure>> {
    const attempt = this.#restoreAttempt;
    try {
      const sdk = await import('matrix-js-sdk');
      const client = sdk.createClient({ baseUrl: this.#baseUrl, localTimeoutMs: 8_000 });
      const flows = await client.loginFlows();
      if (attempt !== this.#restoreAttempt) return err(supersededMatrixSession());
      if (!flows.flows.some((flow) => flow.type === 'm.login.sso')) {
        return err(failure('matrix', 'matrix.sso_unavailable', false, false));
      }
      const previousPath = this.#sessionStorage.getItem(MATRIX_RETURN_PATH_KEY);
      this.#sessionStorage.setItem(
        MATRIX_RETURN_PATH_KEY,
        safeReturnPath(returnPath === '/connect' ? (previousPath ?? returnPath) : returnPath),
      );
      const callback = new URL('/connect', this.#url().origin);
      const loginUrl = client.getSsoLoginUrl(
        callback.toString(),
        'sso',
        undefined,
        sdk.SSOAction.LOGIN,
      );
      this.#navigate(loginUrl);
      return ok({ kind: 'browser-navigation' });
    } catch {
      return err(failure('matrix', 'matrix.sso_start_failed', !this.#online(), true));
    }
  }

  async exchangeAuthenticationGrant(
    loginToken: string,
    returnPath: string,
  ): Promise<Result<void, SessionFailure>> {
    if (!isValidLoginToken(loginToken) || !isSafeReturnPath(returnPath)) {
      return err(failure('matrix', 'matrix.invalid_desktop_authentication_grant', false, false));
    }
    const exchanged = await this.#exchangeLoginToken(loginToken);
    if (!exchanged.ok) {
      return exchanged;
    }
    this.#freshAuthenticationReturnPath = returnPath;
    return ok(undefined);
  }

  async restore(expectedUserId: string): Promise<Result<MatrixRestoreOutcome, SessionFailure>> {
    const restoring = this.#restoreSession(expectedUserId);
    this.#pendingRestores.add(restoring);
    try {
      return await restoring;
    } finally {
      this.#pendingRestores.delete(restoring);
    }
  }

  async #restoreSession(
    expectedUserId: string,
  ): Promise<Result<MatrixRestoreOutcome, SessionFailure>> {
    if (
      this.#logoutInProgress ||
      this.#pendingLogout !== null ||
      this.#pendingRevocation !== null
    ) {
      return err(failure('matrix', 'matrix.logout_incomplete', false, true));
    }
    const attempt = ++this.#restoreAttempt;
    // 这里还没进集合的只有这一次；之前的都已被取代，可能还拿着加密库的锁。
    const earlier = [...this.#pendingRestores];
    this.#stopRestoringClients();
    this.#activeConnection?.disconnect();
    this.#activeConnection = null;
    this.#cryptoLease?.release();
    this.#cryptoLease = null;
    this.#secretStorageKeys.clear();
    this.#onClientChange(null);

    const capturedTokenResult = this.#consumeLoginToken();
    if (!capturedTokenResult.ok) {
      return capturedTokenResult;
    }
    const capturedToken = capturedTokenResult.value;
    const sessionResult =
      capturedToken === null
        ? await this.#sessions.load()
        : await this.#exchangeLoginToken(capturedToken);
    if (!sessionResult.ok) {
      return sessionResult;
    }
    if (attempt !== this.#restoreAttempt) return err(supersededMatrixSession());
    if (sessionResult.value === null) {
      return ok({ kind: 'authentication-required' });
    }
    if (sessionResult.value.userId !== expectedUserId) {
      await this.#revokeSession(sessionResult.value);
      const cleared = await this.#sessions.clear();
      if (!cleared.ok) return cleared;
      return err(failure('identity', 'matrix.identity_mismatch', false, false));
    }
    const returnPath =
      capturedToken === null
        ? this.#takeFreshAuthenticationReturnPath()
        : this.#consumeReturnPath();

    const sdk = await import('matrix-js-sdk');
    // 加密模块（含 WASM 胶水代码）先下载起来，和后面的本地存储、whoami 同时进行，
    // 不再等它们做完才开始；没登录的访客走不到这里，不会多下载。
    prefetchMatrixCrypto();
    const store =
      this.#indexedDB === undefined
        ? new sdk.MemoryStore({
            ...(this.#localStorage === undefined ? {} : { localStorage: this.#localStorage }),
          })
        : new sdk.IndexedDBStore({
            dbName: `agent-room-sync-${stableHash(expectedUserId)}`,
            indexedDB: this.#indexedDB,
            ...(this.#localStorage === undefined ? {} : { localStorage: this.#localStorage }),
          });
    const epoch = this.#sessions.epoch;
    let candidate: MatrixClient | undefined;
    let connected = false;
    let lease: MatrixCryptoLease | null = null;
    try {
      const session = sessionResult.value;
      if (this.#indexedDB !== undefined) {
        // 锁要是还在本页被取代的那次手里，等它放开，别报成另一个窗口占着。
        if (
          (await settleWithin(Promise.allSettled(earlier), RESTORE_HANDOFF_WAIT_MS)) === 'pending'
        ) {
          return err(failure('matrix', 'matrix.restore_stuck', false, true));
        }
        if (attempt !== this.#restoreAttempt) return err(supersededMatrixSession());
        const acquired = await acquireMatrixCryptoLease(
          `agent-room.matrix:${JSON.stringify([this.#baseUrl, session.userId, session.deviceId])}`,
          navigator.locks,
        );
        if (!acquired.ok) return acquired;
        lease = acquired.value;
        if (attempt !== this.#restoreAttempt) return err(supersededMatrixSession());
      }
      const refreshClient = sdk.createClient({ baseUrl: this.#baseUrl, localTimeoutMs: 8_000 });
      const lifecycleLog = new MatrixLifecycleLogger();
      const client = sdk.createClient({
        accessToken: session.accessToken,
        baseUrl: this.#baseUrl,
        cryptoCallbacks: this.#secretStorageKeys.callbacks,
        deviceId: session.deviceId,
        // 重建签名身份时，上传新签名公钥的那一个请求改送控制面代传（ADR 0011）。
        ...(this.#encryptionKeyEscrow === undefined
          ? {}
          : {
              fetchFn: routeCrossSigningUpload(
                globalThis.fetch.bind(globalThis),
                this.#encryptionKeyEscrow,
              ),
            }),
        localTimeoutMs: 20_000,
        logger: lifecycleLog.logger,
        ...(session.refreshToken === undefined ? {} : { refreshToken: session.refreshToken }),
        store,
        timelineSupport: true,
        tokenRefreshFunction: async (refreshToken) => {
          let expiry: Date | undefined;
          const rotated = await this.#sessions.rotate(
            epoch,
            () => attempt === this.#restoreAttempt,
            async () => {
              const refreshed: MatrixRefreshResponse =
                await refreshClient.refreshToken(refreshToken);
              const lifetime = refreshed.expires_in_ms;
              expiry =
                typeof lifetime === 'number' && Number.isSafeInteger(lifetime) && lifetime >= 0
                  ? new Date(Date.now() + lifetime)
                  : undefined;
              return {
                accessToken: refreshed.access_token,
                deviceId: session.deviceId,
                refreshToken: refreshed.refresh_token ?? refreshToken,
                userId: session.userId,
                version: 1,
              };
            },
          );
          if (!rotated.ok) throw new MatrixPersistenceError(rotated.error);
          return {
            accessToken: rotated.value.accessToken,
            ...(expiry === undefined ? {} : { expiry }),
            ...(rotated.value.refreshToken === undefined
              ? {}
              : { refreshToken: rotated.value.refreshToken }),
          };
        },
        userId: session.userId,
        verificationMethods: [MATRIX_SAS_VERIFICATION_METHOD],
      });
      candidate = client;
      this.#restoringClients.add(client);
      this.#clientLogs.set(client, lifecycleLog);
      await store.startup();
      const whoAmI = whoAmISchema.safeParse(await client.whoami());
      if (attempt !== this.#restoreAttempt) {
        return err(supersededMatrixSession());
      }
      if (
        !whoAmI.success ||
        whoAmI.data.user_id !== expectedUserId ||
        (whoAmI.data.device_id !== undefined && whoAmI.data.device_id !== session.deviceId)
      ) {
        await discardMatrixClient(client, lifecycleLog);
        const cleared = await this.#sessions.clear();
        if (!cleared.ok) return cleared;
        return err(failure('identity', 'matrix.identity_mismatch', false, false));
      }

      const persistent = this.#indexedDB !== undefined;
      // 加密库起来以前先记下服务器上这台设备的签名公钥，起来以后跟本机的比。
      // 只在本机加密库持久保存时比：存在内存里的每次都是一套新密钥。
      const recorded = persistent
        ? await recordedDeviceSigningKey(client, session.userId, session.deviceId)
        : undefined;
      if (attempt !== this.#restoreAttempt) return err(supersededMatrixSession());
      try {
        const cryptoApi = await import('matrix-js-sdk/lib/crypto-api/index.js');
        await initializeMatrixCrypto(client, {
          databasePrefix: matrixCryptoDatabasePrefix(session.userId, session.deviceId),
          isolationMode: new cryptoApi.OnlySignedDevicesIsolationMode(),
          persistent,
        });
      } catch {
        return err(failure('matrix', 'matrix.crypto_initialization_failed', !this.#online(), true));
      }

      if (attempt !== this.#restoreAttempt) {
        return err(supersededMatrixSession());
      }
      if (await deviceKeysReplaced(client, recorded)) {
        return await this.#retireDevice(client, lifecycleLog, session);
      }
      if (attempt !== this.#restoreAttempt) return err(supersededMatrixSession());
      const connection = this.#createConnection(client, sdk.ClientEvent.Sync, sdk.SyncState);
      this.#activeConnection = connection;
      this.#cryptoLease = lease;
      this.#onClientChange(client);
      connected = true;
      if (persistent) this.#requestPersistentStorage();
      return ok({
        connection,
        kind: 'connected',
        ...(returnPath === undefined ? {} : { returnPath }),
      });
    } catch (error) {
      if (attempt !== this.#restoreAttempt) return err(supersededMatrixSession());
      if (this.#sessions.failure !== null) return err(this.#sessions.failure);
      if (error instanceof MatrixPersistenceError) return err(error.failure);
      if (isUnauthorized(error)) {
        const cleared = await this.#sessions.clear();
        if (!cleared.ok) return cleared;
        try {
          await store.deleteAllData();
        } catch {
          return err(failure('browser', 'browser.matrix_cache_clear_failed', false, true));
        }
        return ok({ kind: 'authentication-required' });
      }
      return err(failure('matrix', 'matrix.restore_failed', !this.#online(), true));
    } finally {
      if (candidate !== undefined) this.#restoringClients.delete(candidate);
      if (!connected && candidate !== undefined) {
        stopMatrixClient(candidate, this.#clientLogs.get(candidate));
        this.#retainClientForLogout(candidate, sdk.ClientEvent.Sync, sdk.SyncState, lease);
      }
      if (lease !== this.#cryptoLease) lease?.release();
    }
  }

  disconnect(): void {
    ++this.#restoreAttempt;
    this.#stopRestoringClients();
    this.#activeConnection?.disconnect();
    this.#activeConnection = null;
    this.#cryptoLease?.release();
    this.#cryptoLease = null;
    this.#secretStorageKeys.clear();
    this.#onClientChange(null);
  }

  async logout(): Promise<Result<void, SessionFailure>> {
    this.#logoutInProgress = true;
    const lease = this.#cryptoLease;
    this.#cryptoLease = null;
    try {
      return await this.#logoutSession();
    } finally {
      lease?.release();
      this.#releaseCryptoLease();
      this.#logoutInProgress = false;
    }
  }

  async #logoutSession(): Promise<Result<void, SessionFailure>> {
    let active = this.#pendingLogout ?? this.#activeConnection;
    this.disconnect();
    this.#pendingLogout = active;
    this.#activeConnection = null;
    active?.disconnect();
    // SDK 加密初始化会启动后台请求；先等被取代的恢复完成并中止请求，再撤销令牌。
    const restores = await Promise.allSettled(this.#pendingRestores);
    active = this.#pendingLogout ?? active;
    active?.disconnect();
    const stored = active === null ? await this.#sessions.load() : ok(null);
    if (stored.ok) this.#pendingRevocation ??= stored.value;
    const cleared = await this.#sessions.clear();
    const returnPathCleared = this.#clearReturnPath();
    const remote =
      active !== null
        ? await active.logout()
        : this.#pendingRevocation !== null
          ? await this.#revokeSession(this.#pendingRevocation)
          : stored.ok
            ? ok(undefined)
            : stored;
    if (remote.ok) {
      this.#pendingLogout = null;
      this.#pendingRevocation = null;
    }
    if (!cleared.ok) return cleared;
    if (remote.ok && restores.some((result) => result.status === 'rejected')) {
      return err(failure('matrix', 'matrix.logout_failed', !this.#online(), true));
    }
    return returnPathCleared.ok ? remote : returnPathCleared;
  }

  #createConnection(
    client: MatrixClient,
    syncEvent: ClientEvent.Sync,
    syncState: typeof SyncState,
  ): BrowserMatrixConnection {
    return new BrowserMatrixConnection(
      client,
      syncEvent,
      syncState,
      this.#online,
      this.#syncTimeoutMs,
      this.#onClientActivity,
      () => this.#sessions.failure,
      this.#cryptoCleanup,
      this.#ensureEncryption,
      this.#clientLogs.get(client),
    );
  }

  /**
   * 首次同步之后跑一遍：有服务器保管的钥匙就让这台设备自动签好（`specs/device-signing/design.md`），
   * 没配就只给第一台设备建立身份。失败不挡登录，状态交给界面，界面可以重试。`signal` 中止
   * （退出登录、断开）后做完手头这一步就停下，不算失败。
   */
  readonly #ensureEncryption = async (client: MatrixClient, signal: AbortSignal): Promise<void> => {
    const escrow = this.#encryptionKeyEscrow;
    if (escrow === undefined) {
      await ensureFirstEncryptionIdentity(client);
      return;
    }
    // 每次都重新读：等待的时候可能被叫停了。
    const stopped = (): boolean => signal.aborted;
    const attempt = async (): Promise<void> => {
      if (stopped()) return;
      this.#deviceSigning.set({ kind: 'working' });
      try {
        await ensureDeviceSigned(client, escrow, this.#secretStorageKeys, signal);
        this.#deviceSigning.set({ kind: 'ready' });
      } catch {
        if (stopped()) return;
        this.#deviceSigning.set({ kind: 'failed' });
      }
      if (stopped()) return;
      // 签好了，依赖安全状态的界面重新读一遍。
      this.#onClientActivity(client);
    };
    this.#deviceSigning.setRetry(() => {
      void attempt();
    });
    await attempt();
  };

  /**
   * 本机加密存储丢过，加密库用同一个设备号新建了一套密钥。Agent 的加密库不认“同一设备号换了密钥”，
   * 这台设备再也拿不到房间密钥，消息全都解不开。注销它（服务器上连设备一起删掉），清掉本机的会话和存储，
   * 让界面重新登录、换一个新设备号；新设备照常自动签名、找回历史。
   */
  async #retireDevice(
    client: MatrixClient,
    lifecycleLog: MatrixLifecycleLogger,
    session: StoredMatrixSession,
  ): Promise<Result<MatrixRestoreOutcome, SessionFailure>> {
    lifecycleLog.beginShutdown();
    try {
      await client.logout(true);
    } catch {
      // 服务器上留下这台旧设备也不影响新设备；本机照样清掉，免得接着用它。
    }
    client.stopClient();
    const prefix = matrixCryptoDatabasePrefix(session.userId, session.deviceId);
    if (!(await clearDeviceStores(client, prefix, this.#cryptoCleanup))) {
      this.#cryptoCleanup.defer(prefix);
    }
    const cleared = await this.#sessions.clear();
    return cleared.ok ? ok({ kind: 'authentication-required' }) : cleared;
  }

  #requestPersistentStorage(): void {
    if (this.#storagePersistenceRequested) return;
    this.#storagePersistenceRequested = true;
    void this.#persistStorage().catch(() => false);
  }

  #releaseCryptoLease(): void {
    this.#cryptoLease?.release();
    this.#cryptoLease = null;
  }

  /** 被取代的恢复停下手里的请求，随即失败收尾，放开加密库的锁；收尾时它自己从集合里退出。 */
  #stopRestoringClients(): void {
    for (const client of this.#restoringClients) {
      stopMatrixClient(client, this.#clientLogs.get(client));
    }
  }

  #retainClientForLogout(
    client: MatrixClient,
    syncEvent: ClientEvent.Sync,
    syncState: typeof SyncState,
    lease: MatrixCryptoLease | null,
  ): void {
    if (!this.#logoutInProgress || this.#pendingLogout !== null) return;
    this.#pendingLogout = this.#createConnection(client, syncEvent, syncState);
    this.#cryptoLease = lease;
  }

  #consumeLoginToken(): Result<string | null, SessionFailure> {
    const current = this.#url();
    const token = current.searchParams.get('loginToken');
    if (token === null) {
      return ok(null);
    }
    current.searchParams.delete('loginToken');
    this.#replaceHistory(`${current.pathname}${current.search}${current.hash}`);
    return current.pathname === '/connect'
      ? ok(token)
      : err(failure('matrix', 'matrix.invalid_sso_callback_path', false, false));
  }

  #consumeReturnPath(): string | undefined {
    try {
      const path = this.#sessionStorage.getItem(MATRIX_RETURN_PATH_KEY);
      this.#sessionStorage.removeItem(MATRIX_RETURN_PATH_KEY);
      return path === null ? undefined : safeReturnPath(path);
    } catch {
      return undefined;
    }
  }

  #takeFreshAuthenticationReturnPath(): string | undefined {
    const returnPath = this.#freshAuthenticationReturnPath;
    this.#freshAuthenticationReturnPath = undefined;
    return returnPath;
  }

  async #exchangeLoginToken(
    loginToken: string,
  ): Promise<Result<StoredMatrixSession, SessionFailure>> {
    const epoch = this.#sessions.beginSession();
    try {
      const sdk = await import('matrix-js-sdk');
      const client = sdk.createClient({ baseUrl: this.#baseUrl, localTimeoutMs: 8_000 });
      const decoded = loginResponseSchema.safeParse(
        await client.loginRequest({
          initial_device_display_name: this.#deviceDisplayName,
          refresh_token: true,
          token: loginToken,
          type: 'm.login.token',
        }),
      );
      if (!decoded.success) {
        return err(failure('matrix', 'matrix.invalid_login_response', false, false));
      }
      const session: StoredMatrixSession = {
        accessToken: decoded.data.access_token,
        deviceId: decoded.data.device_id,
        ...(decoded.data.refresh_token === undefined
          ? {}
          : { refreshToken: decoded.data.refresh_token }),
        userId: decoded.data.user_id,
        version: 1,
      };
      const parsed = storedMatrixSessionSchema.safeParse(session);
      if (!parsed.success)
        return err(failure('matrix', 'matrix.invalid_login_response', false, false));
      const persisted = await this.#sessions.save(parsed.data, epoch);
      return persisted.ok ? ok(session) : persisted;
    } catch {
      return err(failure('matrix', 'matrix.login_exchange_failed', !this.#online(), true));
    }
  }

  #clearReturnPath(): Result<void, SessionFailure> {
    try {
      this.#sessionStorage.removeItem(MATRIX_RETURN_PATH_KEY);
      return ok(undefined);
    } catch {
      return err(failure('browser', 'browser.session_storage_unavailable', false, true));
    }
  }

  async #revokeSession(session: StoredMatrixSession): Promise<Result<void, SessionFailure>> {
    try {
      const sdk = await import('matrix-js-sdk');
      const client = sdk.createClient({
        accessToken: session.accessToken,
        baseUrl: this.#baseUrl,
        deviceId: session.deviceId,
        localTimeoutMs: 8_000,
        userId: session.userId,
      });
      await client.logout(true);
      return ok(undefined);
    } catch (error) {
      return isUnauthorized(error)
        ? ok(undefined)
        : err(failure('matrix', 'matrix.logout_failed', !this.#online(), true));
    }
  }
}

class MatrixPersistenceError extends Error {
  constructor(readonly failure: SessionFailure) {
    super(failure.code);
  }
}

function prefetchMatrixCrypto(): void {
  // 只是预热模块缓存：失败了等到真正初始化加密时再按原来的方式报错。
  void import('matrix-js-sdk/lib/rust-crypto/index.js').catch(ignorePrefetchFailure);
}

function ignorePrefetchFailure(error: unknown): void {
  void error;
}

type MatrixCryptoClient = Pick<MatrixClient, 'getCrypto' | 'initRustCrypto'>;

export type MatrixCryptoInitialization = {
  readonly databasePrefix: string;
  readonly isolationMode: DeviceIsolationMode;
  readonly persistent: boolean;
};

export async function initializeMatrixCrypto(
  client: MatrixCryptoClient,
  initialization: MatrixCryptoInitialization,
): Promise<void> {
  await client.initRustCrypto({
    cryptoDatabasePrefix: initialization.databasePrefix,
    useIndexedDB: initialization.persistent,
  });
  const crypto = client.getCrypto();
  if (crypto === undefined) {
    throw new Error('Matrix Rust Crypto 初始化完成后仍不可用。');
  }
  crypto.setTrustCrossSignedDevices(true);
  crypto.setDeviceIsolationMode(initialization.isolationMode);
}

/** 服务器上记着的这台设备的签名公钥。新设备还没传过密钥时没有；问不到也当没有，不挡恢复。 */
export async function recordedDeviceSigningKey(
  client: Pick<MatrixClient, 'downloadKeysForUsers'>,
  userId: string,
  deviceId: string,
): Promise<string | undefined> {
  try {
    const response = keysQuerySchema.safeParse(await client.downloadKeysForUsers([userId]));
    if (!response.success) return undefined;
    return response.data.device_keys?.[userId]?.[deviceId]?.keys[`ed25519:${deviceId}`];
  } catch {
    return undefined;
  }
}

/**
 * 本机加密库的签名公钥和它起来以前服务器上记着的不一样：本机加密存储丢过，加密库用同一个设备号
 * 新建了一套密钥。服务器上没记着（新设备）或者本机的读不出来时不算。
 */
export async function deviceKeysReplaced(
  client: Pick<MatrixClient, 'getCrypto'>,
  recorded: string | undefined,
): Promise<boolean> {
  if (recorded === undefined) return false;
  try {
    const own = await client.getCrypto()?.getOwnDeviceKeys();
    return own !== undefined && own.ed25519 !== recorded;
  } catch {
    return false;
  }
}

/**
 * 请浏览器别在空间紧张时清掉本站数据：本机加密存储一丢，这台设备就只能换个设备号重新登录。
 * 已经是持久的就不再请求；浏览器不支持时什么也不做。
 */
async function requestPersistentStorage(): Promise<boolean> {
  const storage = (globalThis.navigator as Navigator | undefined)?.storage as
    Partial<StorageManager> | undefined;
  if (storage?.persist === undefined) return false;
  if ((await storage.persisted?.()) === true) return true;
  return await storage.persist();
}

function ignoreClientChange(client: MatrixClient | null): void {
  void client;
}

function ignoreClientActivity(client: MatrixClient): void {
  void client;
}

class BrowserMatrixConnection implements MatrixConnection {
  readonly deviceId: string;
  readonly userId: string;
  readonly #client: MatrixClient;
  readonly #onClientActivity: (client: MatrixClient) => void;
  readonly #online: () => boolean;
  readonly #syncEvent: ClientEvent.Sync;
  readonly #syncState: typeof SyncState;
  readonly #syncTimeoutMs: number;
  readonly #persistenceFailure: () => SessionFailure | null;
  readonly #cryptoCleanup: MatrixCryptoStoreCleanup;
  #observingActivity = true;
  #started = false;
  #identityEnsured = false;
  #revoked = false;
  #storesCleared = false;
  /** 正在跑的自动签名；退出登录前先叫停它、等它停下。 */
  #encryption: Promise<void> | null = null;
  readonly #stopEncryption = new AbortController();

  constructor(
    client: MatrixClient,
    syncEvent: ClientEvent.Sync,
    syncState: typeof SyncState,
    online: () => boolean,
    syncTimeoutMs: number,
    onClientActivity: (client: MatrixClient) => void,
    persistenceFailure: () => SessionFailure | null,
    cryptoCleanup: MatrixCryptoStoreCleanup,
    private readonly ensureEncryption: (client: MatrixClient, signal: AbortSignal) => Promise<void>,
    private readonly lifecycleLog: MatrixLifecycleLogger | undefined,
  ) {
    this.#client = client;
    this.#syncEvent = syncEvent;
    this.#syncState = syncState;
    this.#onClientActivity = onClientActivity;
    this.#online = online;
    this.#syncTimeoutMs = syncTimeoutMs;
    this.#persistenceFailure = persistenceFailure;
    this.#cryptoCleanup = cryptoCleanup;
    this.deviceId = client.getDeviceId() ?? 'unknown-device';
    this.userId = client.getUserId() ?? 'unknown-user';
    this.#client.on(this.#syncEvent, this.#handleClientActivity);
  }

  disconnect(): void {
    this.#stopEncryption.abort();
    this.#stopObservingActivity();
    stopMatrixClient(this.#client, this.lifecycleLog);
  }

  observe(listener: (status: MatrixConnectionStatus) => void): () => void {
    const onSync = (state: SyncState): void => {
      const mapped = matrixStatus(state, this.#syncState);
      if (mapped !== null) {
        listener(mapped);
      }
    };
    this.#client.on(this.#syncEvent, onSync);
    return () => {
      this.#client.removeListener(this.#syncEvent, onSync);
    };
  }

  async waitUntilPrepared(): Promise<Result<void, SessionFailure>> {
    const persistenceFailure = this.#persistenceFailure();
    if (persistenceFailure !== null) return err(persistenceFailure);
    const current = this.#client.getSyncState();
    if (current === this.#syncState.Prepared || current === this.#syncState.Syncing) {
      this.#ensureEncryptionIdentity();
      return ok(undefined);
    }
    return await new Promise((resolve) => {
      let settled = false;
      const finish = (result: Result<void, SessionFailure>): void => {
        if (settled) {
          return;
        }
        settled = true;
        window.clearTimeout(timeout);
        this.#client.removeListener(this.#syncEvent, onSync);
        resolve(result);
      };
      const onSync = (state: SyncState): void => {
        if (state === this.#syncState.Prepared || state === this.#syncState.Syncing) {
          this.#ensureEncryptionIdentity();
          finish(ok(undefined));
        } else if (state === this.#syncState.Error || state === this.#syncState.Stopped) {
          finish(
            err(
              this.#persistenceFailure() ??
                failure('matrix', 'matrix.initial_sync_failed', !this.#online(), true),
            ),
          );
        }
      };
      const timeout = window.setTimeout(() => {
        finish(err(failure('matrix', 'matrix.initial_sync_timeout', !this.#online(), true)));
      }, this.#syncTimeoutMs);
      this.#client.on(this.#syncEvent, onSync);
      if (!this.#started) {
        this.#started = true;
        void this.#client.startClient({ initialSyncLimit: 20 }).catch(() => {
          finish(
            err(
              this.#persistenceFailure() ??
                failure('matrix', 'matrix.initial_sync_failed', !this.#online(), true),
            ),
          );
        });
      }
    });
  }

  /** 首次同步完成后本机设备密钥已上传，此时让这台设备签好（全新账户先建立签名身份）。 */
  #ensureEncryptionIdentity(): void {
    if (this.#identityEnsured) return;
    this.#identityEnsured = true;
    this.#encryption = this.ensureEncryption(this.#client, this.#stopEncryption.signal);
  }

  async logout(): Promise<Result<void, SessionFailure>> {
    this.lifecycleLog?.beginShutdown();
    // 自动签名还在跑时先叫停，等它做完手头这一步：令牌作废以后它再发的请求都会被拒绝（401）。
    this.#stopEncryption.abort();
    if (this.#encryption !== null) await settleWithin(this.#encryption, SIGNING_STOP_WAIT_MS);
    let remoteResult: Result<void, SessionFailure> = ok(undefined);
    if (!this.#revoked) {
      try {
        await this.#client.logout(true);
        this.#revoked = true;
      } catch (error) {
        if (isUnauthorized(error)) this.#revoked = true;
        else remoteResult = err(failure('matrix', 'matrix.logout_failed', !this.#online(), true));
      }
    }

    this.#stopObservingActivity();
    this.#client.stopClient();
    if (!this.#storesCleared) {
      const prefix = matrixCryptoDatabasePrefix(this.userId, this.deviceId);
      if (!(await clearDeviceStores(this.#client, prefix, this.#cryptoCleanup))) {
        return remoteResult.ok
          ? err(failure('browser', 'browser.matrix_cache_clear_failed', false, true))
          : remoteResult;
      }
      this.#storesCleared = true;
    }
    return remoteResult;
  }

  readonly #handleClientActivity = (): void => {
    this.#onClientActivity(this.#client);
  };

  #stopObservingActivity(): void {
    if (!this.#observingActivity) {
      return;
    }
    this.#observingActivity = false;
    this.#client.removeListener(this.#syncEvent, this.#handleClientActivity);
  }
}

function matrixStatus(state: SyncState, states: typeof SyncState): MatrixConnectionStatus | null {
  const mapping = new Map<SyncState, MatrixConnectionStatus>([
    [states.Prepared, 'ready'],
    [states.Syncing, 'ready'],
    [states.Catchup, 'reconnecting'],
    [states.Reconnecting, 'reconnecting'],
    [states.Error, 'failed'],
    [states.Stopped, 'stopped'],
  ]);
  return mapping.get(state) ?? null;
}

function safeReturnPath(path: string): string {
  return isSafeReturnPath(path) ? path : '/connect';
}

function isSafeReturnPath(path: string): boolean {
  return (
    path.startsWith('/') &&
    !path.startsWith('//') &&
    !path.includes('\\') &&
    path.length <= 2_048 &&
    !/\p{Cc}/u.test(path)
  );
}

function isValidLoginToken(token: string): boolean {
  return token.length > 0 && token.length <= MAX_LOGIN_TOKEN_LENGTH && !/\p{Cc}/u.test(token);
}

/** 等 `work` 最多 `ms` 毫秒：完成、失败，或者还没结果（仍在进行，不会被取消）。 */
async function settleWithin(
  work: Promise<unknown>,
  ms: number,
): Promise<'done' | 'failed' | 'pending'> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const pending = new Promise<'pending'>((resolve) => {
    timer = setTimeout(() => {
      resolve('pending');
    }, ms);
  });
  try {
    return await Promise.race([
      work.then(
        () => 'done' as const,
        () => 'failed' as const,
      ),
      pending,
    ]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * 删掉同步缓存和这台设备的本机加密库，删除失败时返回 false。加密库仍被停下的 Rust 加密模块占着时不一直等：
 * 删除会在它放手后自己完成；页面要是先关了，下次启动再删（见 {@link MatrixCryptoStoreCleanup}）。
 */
async function clearDeviceStores(
  client: MatrixClient,
  prefix: string,
  cleanup: MatrixCryptoStoreCleanup,
): Promise<boolean> {
  const clearing = client.clearStores({ cryptoDatabasePrefix: prefix });
  const outcome = await settleWithin(clearing, CRYPTO_STORE_CLEAR_WAIT_MS);
  if (outcome === 'failed') return false;
  if (outcome === 'pending') {
    cleanup.defer(prefix);
    void clearing.then(
      () => {
        cleanup.done(prefix);
      },
      () => undefined,
    );
  }
  return true;
}

function matrixCryptoDatabasePrefix(userId: string, deviceId: string): string {
  return `agent-room-crypto-${stableHash(`${userId}\u0000${deviceId}`)}`;
}

function stableHash(value: string): string {
  let hash = 2_166_136_261;
  for (const character of value) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16_777_619);
  }
  return (hash >>> 0).toString(16).padStart(8, '0');
}

function isUnauthorized(error: unknown): boolean {
  return (
    typeof error === 'object' && error !== null && 'httpStatus' in error && error.httpStatus === 401
  );
}

function stopMatrixClient(client: MatrixClient, lifecycleLog?: MatrixLifecycleLogger): void {
  lifecycleLog?.beginShutdown();
  client.stopClient();
  client.http.abort();
}

async function discardMatrixClient(
  client: MatrixClient,
  lifecycleLog: MatrixLifecycleLogger,
): Promise<void> {
  lifecycleLog.beginShutdown();
  try {
    await client.logout(true);
  } catch {
    // 无论远端撤销是否可达，本地凭据和同步缓存都必须继续清理。
  }
  client.stopClient();
  try {
    await client.clearStores();
  } catch {
    // 身份不匹配已经失败关闭；缓存不可访问时不会继续使用该客户端。
  }
}
