import type { ClientEvent, MatrixClient, SyncState } from 'matrix-js-sdk';
import { z } from 'zod';

import { failure } from '@/features/session/adapters/control-plane-client';
import type {
  MatrixConnection,
  MatrixConnectionStatus,
  MatrixGateway,
  MatrixRestoreOutcome,
  SessionFailure,
} from '@/features/session/domain/session';
import { err, ok, type Result } from '@/shared/result';

const MATRIX_SESSION_KEY = 'agent-room.matrix-session.v1';
const MATRIX_RETURN_PATH_KEY = 'agent-room.matrix-return-path.v1';

const storedSessionSchema = z
  .object({
    accessToken: z.string().min(1),
    deviceId: z.string().min(1).max(255),
    refreshToken: z.string().min(1).optional(),
    userId: z.string().regex(/^@[^:]+:.+$/u),
    version: z.literal(1),
  })
  .strict();

type StoredMatrixSession = z.output<typeof storedSessionSchema>;

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

export type MatrixWebGatewayOptions = {
  readonly baseUrl: string;
  readonly indexedDB?: IDBFactory;
  readonly localStorage?: Storage;
  readonly navigate?: (url: string) => void;
  readonly online?: () => boolean;
  readonly replaceHistory?: (url: string) => void;
  readonly sessionStorage?: Storage;
  readonly syncTimeoutMs?: number;
  readonly url?: () => URL;
};

export class MatrixWebGateway implements MatrixGateway {
  readonly #baseUrl: string;
  readonly #indexedDB: IDBFactory | undefined;
  readonly #localStorage: Storage | undefined;
  readonly #navigate: (url: string) => void;
  readonly #online: () => boolean;
  readonly #replaceHistory: (url: string) => void;
  readonly #sessionStorage: Storage;
  readonly #syncTimeoutMs: number;
  readonly #url: () => URL;
  #activeConnection: BrowserMatrixConnection | null = null;

  constructor({
    baseUrl,
    indexedDB = window.indexedDB,
    localStorage = window.localStorage,
    navigate = (url) => {
      window.location.assign(url);
    },
    online = () => window.navigator.onLine,
    replaceHistory = (url) => {
      window.history.replaceState(window.history.state, '', url);
    },
    sessionStorage = window.sessionStorage,
    syncTimeoutMs = 20_000,
    url = () => new URL(window.location.href),
  }: MatrixWebGatewayOptions) {
    this.#baseUrl = baseUrl;
    this.#indexedDB = indexedDB;
    this.#localStorage = localStorage;
    this.#navigate = navigate;
    this.#online = online;
    this.#replaceHistory = replaceHistory;
    this.#sessionStorage = sessionStorage;
    this.#syncTimeoutMs = syncTimeoutMs;
    this.#url = url;
  }

  async beginAuthentication(returnPath: string): Promise<Result<void, SessionFailure>> {
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
      return ok(undefined);
    } catch {
      return err(failure('matrix', 'matrix.sso_start_failed', !this.#online(), true));
    }
  }

  async restore(expectedUserId: string): Promise<Result<MatrixRestoreOutcome, SessionFailure>> {
    this.#activeConnection?.disconnect();
    this.#activeConnection = null;

    const capturedTokenResult = this.#consumeLoginToken();
    if (!capturedTokenResult.ok) {
      return capturedTokenResult;
    }
    const capturedToken = capturedTokenResult.value;
    const sessionResult =
      capturedToken === null
        ? this.#readStoredSession()
        : await this.#exchangeLoginToken(capturedToken);
    if (!sessionResult.ok) {
      return sessionResult;
    }
    if (sessionResult.value === null) {
      return ok({ kind: 'authentication-required' });
    }
    if (sessionResult.value.userId !== expectedUserId) {
      await this.#revokeSession(sessionResult.value);
      this.#clearStoredSession();
      return err(failure('identity', 'matrix.identity_mismatch', false, false));
    }

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
    try {
      await store.startup();
      const session = sessionResult.value;
      const refreshClient = sdk.createClient({ baseUrl: this.#baseUrl, localTimeoutMs: 8_000 });
      const client = sdk.createClient({
        accessToken: session.accessToken,
        baseUrl: this.#baseUrl,
        deviceId: session.deviceId,
        localTimeoutMs: 20_000,
        ...(session.refreshToken === undefined ? {} : { refreshToken: session.refreshToken }),
        store,
        timelineSupport: true,
        tokenRefreshFunction: async (refreshToken) => {
          const refreshed = await refreshClient.refreshToken(refreshToken);
          const updated: StoredMatrixSession = {
            accessToken: refreshed.access_token,
            deviceId: session.deviceId,
            refreshToken: refreshed.refresh_token,
            userId: session.userId,
            version: 1,
          };
          this.#writeStoredSession(updated);
          return {
            accessToken: refreshed.access_token,
            expiry: new Date(Date.now() + refreshed.expires_in_ms),
            refreshToken: refreshed.refresh_token,
          };
        },
        userId: session.userId,
      });
      const whoAmI = whoAmISchema.safeParse(await client.whoami());
      if (!whoAmI.success || whoAmI.data.user_id !== expectedUserId) {
        await discardMatrixClient(client);
        this.#clearStoredSession();
        return err(failure('identity', 'matrix.identity_mismatch', false, false));
      }

      const connection = new BrowserMatrixConnection(
        client,
        sdk.ClientEvent.Sync,
        sdk.SyncState,
        this.#online,
        this.#syncTimeoutMs,
      );
      this.#activeConnection = connection;
      const returnPath = capturedToken === null ? undefined : this.#consumeReturnPath();
      return ok({
        connection,
        kind: 'connected',
        ...(returnPath === undefined ? {} : { returnPath }),
      });
    } catch (error) {
      if (isUnauthorized(error)) {
        this.#clearStoredSession();
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

  async logout(): Promise<Result<void, SessionFailure>> {
    const active = this.#activeConnection;
    this.#activeConnection = null;
    this.#clearBrowserState();
    if (active === null) {
      return ok(undefined);
    }
    return await active.logout();
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

  async #exchangeLoginToken(
    loginToken: string,
  ): Promise<Result<StoredMatrixSession, SessionFailure>> {
    try {
      const sdk = await import('matrix-js-sdk');
      const client = sdk.createClient({ baseUrl: this.#baseUrl, localTimeoutMs: 8_000 });
      const decoded = loginResponseSchema.safeParse(
        await client.loginRequest({
          initial_device_display_name: 'Agent Room Web',
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
      const persisted = this.#writeStoredSession(session);
      return persisted.ok ? ok(session) : persisted;
    } catch {
      return err(failure('matrix', 'matrix.login_exchange_failed', !this.#online(), true));
    }
  }

  #readStoredSession(): Result<StoredMatrixSession | null, SessionFailure> {
    try {
      const serialized = this.#sessionStorage.getItem(MATRIX_SESSION_KEY);
      if (serialized === null) {
        return ok(null);
      }
      const parsed = storedSessionSchema.safeParse(JSON.parse(serialized));
      if (!parsed.success) {
        this.#sessionStorage.removeItem(MATRIX_SESSION_KEY);
        return ok(null);
      }
      return ok(parsed.data);
    } catch {
      return err(failure('browser', 'browser.session_storage_unavailable', false, false));
    }
  }

  #writeStoredSession(session: StoredMatrixSession): Result<void, SessionFailure> {
    try {
      this.#sessionStorage.setItem(MATRIX_SESSION_KEY, JSON.stringify(session));
      return ok(undefined);
    } catch {
      return err(failure('browser', 'browser.session_storage_unavailable', false, false));
    }
  }

  #clearStoredSession(): void {
    try {
      this.#sessionStorage.removeItem(MATRIX_SESSION_KEY);
    } catch {
      // The local credential is already inaccessible; no further fallback can expose it.
    }
  }

  #clearBrowserState(): void {
    this.#clearStoredSession();
    try {
      this.#sessionStorage.removeItem(MATRIX_RETURN_PATH_KEY);
    } catch {
      // 两个会话键位于同一不可访问存储中，凭据已经无法被当前页面读取。
    }
  }

  async #revokeSession(session: StoredMatrixSession): Promise<void> {
    try {
      const sdk = await import('matrix-js-sdk');
      const client = sdk.createClient({
        accessToken: session.accessToken,
        baseUrl: this.#baseUrl,
        deviceId: session.deviceId,
        userId: session.userId,
      });
      await client.logout(true);
    } catch {
      // A mismatched local session is cleared even when the remote revoke is unreachable.
    }
  }
}

class BrowserMatrixConnection implements MatrixConnection {
  readonly deviceId: string;
  readonly userId: string;
  readonly #client: MatrixClient;
  readonly #online: () => boolean;
  readonly #syncEvent: ClientEvent.Sync;
  readonly #syncState: typeof SyncState;
  readonly #syncTimeoutMs: number;
  #started = false;

  constructor(
    client: MatrixClient,
    syncEvent: ClientEvent.Sync,
    syncState: typeof SyncState,
    online: () => boolean,
    syncTimeoutMs: number,
  ) {
    this.#client = client;
    this.#syncEvent = syncEvent;
    this.#syncState = syncState;
    this.#online = online;
    this.#syncTimeoutMs = syncTimeoutMs;
    this.deviceId = client.getDeviceId() ?? 'unknown-device';
    this.userId = client.getUserId() ?? 'unknown-user';
  }

  disconnect(): void {
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
          finish(err(failure('matrix', 'matrix.initial_sync_failed', !this.#online(), true)));
        }
      };
      const timeout = window.setTimeout(() => {
        finish(err(failure('matrix', 'matrix.initial_sync_timeout', !this.#online(), true)));
      }, this.#syncTimeoutMs);
      this.#client.on(this.#syncEvent, onSync);
      if (!this.#started) {
        this.#started = true;
        void this.#client.startClient({ initialSyncLimit: 20 }).catch(() => {
          finish(err(failure('matrix', 'matrix.initial_sync_failed', !this.#online(), true)));
        });
      }
    });
  }

  async logout(): Promise<Result<void, SessionFailure>> {
    let remoteResult: Result<void, SessionFailure> = ok(undefined);
    try {
      await this.#client.logout(true);
    } catch {
      remoteResult = err(failure('matrix', 'matrix.logout_failed', !this.#online(), true));
    }

    this.#client.stopClient();
    try {
      await this.#client.clearStores();
    } catch {
      return remoteResult.ok
        ? err(failure('browser', 'browser.matrix_cache_clear_failed', false, true))
        : remoteResult;
    }
    return remoteResult;
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
  return path.startsWith('/') && !path.startsWith('//') && !path.includes('\\') ? path : '/connect';
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
