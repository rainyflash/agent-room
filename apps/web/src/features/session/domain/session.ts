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

export type AuthenticationIntent = 'register' | 'sign-in';

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
  beginAuthentication(returnPath: string, intent?: AuthenticationIntent): void;
  logout(): Promise<Result<void, SessionFailure>>;
  readSession(): Promise<Result<WebSession, SessionFailure>>;
};

export type MatrixGateway = {
  beginAuthentication(returnPath: string): Promise<Result<void, SessionFailure>>;
  logout(): Promise<Result<void, SessionFailure>>;
  restore(expectedUserId: string): Promise<Result<MatrixRestoreOutcome, SessionFailure>>;
};

export type BrowserGateway = {
  currentPath(): string;
  isOnline(): boolean;
  replacePath(path: string): void;
};

export type SessionDependencies = {
  readonly browser: BrowserGateway;
  readonly controlPlane: ControlPlaneGateway;
  readonly matrix: MatrixGateway;
};
