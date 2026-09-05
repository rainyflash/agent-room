import type { ClientEvent, MatrixClient, SyncState } from 'matrix-js-sdk';
import type { DeviceIsolationMode } from 'matrix-js-sdk/lib/crypto-api/index.js';
import { z } from 'zod';

import { failure } from '@/features/session/adapters/control-plane-client';
import { BrowserMatrixSessionVault } from './browser-matrix-session-vault';
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
import { MatrixSecretStorageKeyCache } from '@/shared/matrix/matrix-secret-storage-key-cache';
import { err, ok, type Result } from '@/shared/result';

const MATRIX_RETURN_PATH_KEY = 'agent-room.matrix-return-path.v1';
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
  readonly replaceHistory?: (url: string) => void;
  readonly secretStorageKeys?: MatrixSecretStorageKeyCache;
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
  readonly #replaceHistory: (url: string) => void;
  readonly #secretStorageKeys: MatrixSecretStorageKeyCache;
  readonly #sessionStorage: Storage;
  readonly #sessions: MatrixSessionRepository;
  #restoreAttempt = 0;
  readonly #syncTimeoutMs: number;
  readonly #url: () => URL;
  #activeConnection: BrowserMatrixConnection | null = null;
  #pendingLogout: BrowserMatrixConnection | null = null;
  #pendingRevocation: StoredMatrixSession | null = null;
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
    replaceHistory = (url) => {
      window.history.replaceState(window.history.state, '', url);
    },
    secretStorageKeys = new MatrixSecretStorageKeyCache(),
    sessionStorage = window.sessionStorage,
    sessionVault = new BrowserMatrixSessionVault(sessionStorage),
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
    this.#replaceHistory = replaceHistory;
    this.#secretStorageKeys = secretStorageKeys;
    this.#sessionStorage = sessionStorage;
    this.#sessions = new MatrixSessionRepository(sessionVault);
    this.#syncTimeoutMs = syncTimeoutMs;
    this.#url = url;
  }

  async beginAuthentication(
    returnPath: string,
  ): Promise<Result<AuthenticationStartOutcome, SessionFailure>> {
    try {
      const sdk = await import('matrix-js-sdk');
      const client = sdk.createClient({ baseUrl: this.#baseUrl, localTimeoutMs: 8_000 });
      const flows = await client.loginFlows();
      if (!flows.flows.some((flow) => flow.type === 'm.login.sso')) {
        return err(failure('matrix', 'matrix.sso_unavailable', false, false));
      }
      this.#sessionStorage.setItem(MATRIX_RETURN_PATH_KEY, safeReturnPath(returnPath));
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
    if (this.#pendingLogout !== null || this.#pendingRevocation !== null) {
      return err(failure('matrix', 'matrix.logout_incomplete', false, true));
    }
    const attempt = ++this.#restoreAttempt;
    this.#activeConnection?.disconnect();
    this.#activeConnection = null;
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
    try {
      const session = sessionResult.value;
      const refreshClient = sdk.createClient({ baseUrl: this.#baseUrl, localTimeoutMs: 8_000 });
      const client = sdk.createClient({
        accessToken: session.accessToken,
        baseUrl: this.#baseUrl,
        cryptoCallbacks: this.#secretStorageKeys.callbacks,
        deviceId: session.deviceId,
        localTimeoutMs: 20_000,
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
      await store.startup();
      const whoAmI = whoAmISchema.safeParse(await client.whoami());
      if (attempt !== this.#restoreAttempt) {
        client.stopClient();
        return err(supersededMatrixSession());
      }
      if (
        !whoAmI.success ||
        whoAmI.data.user_id !== expectedUserId ||
        (whoAmI.data.device_id !== undefined && whoAmI.data.device_id !== session.deviceId)
      ) {
        await discardMatrixClient(client);
        const cleared = await this.#sessions.clear();
        if (!cleared.ok) return cleared;
        return err(failure('identity', 'matrix.identity_mismatch', false, false));
      }

      try {
        const cryptoApi = await import('matrix-js-sdk/lib/crypto-api/index.js');
        await initializeMatrixCrypto(client, {
          databasePrefix: matrixCryptoDatabasePrefix(session.userId, session.deviceId),
          isolationMode: new cryptoApi.OnlySignedDevicesIsolationMode(),
          persistent: this.#indexedDB !== undefined,
        });
      } catch {
        client.stopClient();
        return err(failure('matrix', 'matrix.crypto_initialization_failed', !this.#online(), true));
      }

      if (attempt !== this.#restoreAttempt) {
        client.stopClient();
        return err(supersededMatrixSession());
      }
      const connection = new BrowserMatrixConnection(
        client,
        sdk.ClientEvent.Sync,
        sdk.SyncState,
        this.#online,
        this.#syncTimeoutMs,
        this.#onClientActivity,
        () => this.#sessions.failure,
      );
      this.#activeConnection = connection;
      this.#onClientChange(client);
      return ok({
        connection,
        kind: 'connected',
        ...(returnPath === undefined ? {} : { returnPath }),
      });
    } catch (error) {
      candidate?.stopClient();
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
    }
  }

  disconnect(): void {
    ++this.#restoreAttempt;
    this.#activeConnection?.disconnect();
    this.#secretStorageKeys.clear();
    this.#onClientChange(null);
  }

  async logout(): Promise<Result<void, SessionFailure>> {
    this.disconnect();
    const active = this.#pendingLogout ?? this.#activeConnection;
    this.#pendingLogout = active;
    this.#activeConnection = null;
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
    return returnPathCleared.ok ? remote : returnPathCleared;
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
  #observingActivity = true;
  #started = false;
  #revoked = false;
  #storesCleared = false;

  constructor(
    client: MatrixClient,
    syncEvent: ClientEvent.Sync,
    syncState: typeof SyncState,
    online: () => boolean,
    syncTimeoutMs: number,
    onClientActivity: (client: MatrixClient) => void,
    persistenceFailure: () => SessionFailure | null,
  ) {
    this.#client = client;
    this.#syncEvent = syncEvent;
    this.#syncState = syncState;
    this.#onClientActivity = onClientActivity;
    this.#online = online;
    this.#syncTimeoutMs = syncTimeoutMs;
    this.#persistenceFailure = persistenceFailure;
    this.deviceId = client.getDeviceId() ?? 'unknown-device';
    this.userId = client.getUserId() ?? 'unknown-user';
    this.#client.on(this.#syncEvent, this.#handleClientActivity);
  }

  disconnect(): void {
    this.#stopObservingActivity();
    this.#client.stopClient();
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

  async logout(): Promise<Result<void, SessionFailure>> {
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
    try {
      if (!this.#storesCleared) {
        await this.#client.clearStores({
          cryptoDatabasePrefix: matrixCryptoDatabasePrefix(this.userId, this.deviceId),
        });
        this.#storesCleared = true;
      }
    } catch {
      return remoteResult.ok
        ? err(failure('browser', 'browser.matrix_cache_clear_failed', false, true))
        : remoteResult;
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

async function discardMatrixClient(client: MatrixClient): Promise<void> {
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
