import type { Result } from '@/shared/result';

export type WebSession = {
  readonly authenticatedAtUnixMs: number;
  readonly displayName: string;
  readonly expiresAtUnixMs: number;
  readonly locale: string;
  readonly matrixUserId: string;
  readonly principalId: string;
  readonly recentlyAuthenticated: boolean;
};

export type FailureBoundary = 'browser' | 'control-plane' | 'identity' | 'matrix';

export type SessionFailure = {
  readonly boundary: FailureBoundary;
  readonly code: string;
  readonly correlationId?: string;
  readonly offline: boolean;
  readonly retryable: boolean;
};

/** `reauthenticate`：要“最近登录过”的操作请人再登录一次，不拿浏览器里已有的登录换桌面登录。 */
export type AuthenticationIntent = 'reauthenticate' | 'register' | 'sign-in';
export type AuthenticationMode = 'automatic' | 'interactive';

export type AuthenticationStartOutcome =
  { readonly kind: 'browser-navigation' } | { readonly kind: 'session-established' };

export type MatrixConnectionStatus = 'ready' | 'reconnecting' | 'failed' | 'stopped';

export type MatrixConnection = {
  readonly deviceId: string;
  readonly userId: string;
  disconnect(): void;
  observe(listener: (status: MatrixConnectionStatus) => void): () => void;
  waitUntilPrepared(): Promise<Result<void, SessionFailure>>;
};

export type MatrixRestoreOutcome =
  | { readonly kind: 'authentication-required' }
  | {
      readonly connection: MatrixConnection;
      readonly kind: 'connected';
      readonly returnPath?: string;
    };

export type ControlPlaneGateway = {
  beginAuthentication(
    returnPath: string,
    intent?: AuthenticationIntent,
  ): Promise<Result<AuthenticationStartOutcome, SessionFailure>>;
  logout(): Promise<Result<void, SessionFailure>>;
  readSession(): Promise<Result<WebSession, SessionFailure>>;
};

export type MatrixGateway = {
  disconnect(): void;
  beginAuthentication(
    returnPath: string,
    mode?: AuthenticationMode,
  ): Promise<Result<AuthenticationStartOutcome, SessionFailure>>;
  logout(): Promise<Result<void, SessionFailure>>;
  restore(expectedUserId: string): Promise<Result<MatrixRestoreOutcome, SessionFailure>>;
};

export type BrowserGateway = {
  currentPath(): string;
  isOnline(): boolean;
  replacePath(path: string): void;
};

/** 控制面说某个操作要最近登录过（`authentication.reauthentication_required`）时通知。 */
export type ReauthenticationNotices = {
  subscribe(listener: () => void): () => void;
};

export type SessionDependencies = {
  readonly browser: BrowserGateway;
  readonly controlPlane: ControlPlaneGateway;
  readonly matrix: MatrixGateway;
  readonly privateState: { clear(): void };
  readonly reauthentication?: ReauthenticationNotices;
};
