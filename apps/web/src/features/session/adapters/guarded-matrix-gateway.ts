import type { AuthenticationMode, MatrixGateway, SessionFailure } from '../domain/session';
import { err, ok, type Result } from '@/shared/result';

/** A navigation survives the page that started it; keep its attempt separate from credentials. */
export class GuardedMatrixGateway implements MatrixGateway {
  readonly #matrix: MatrixGateway;
  readonly #storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
  readonly #key: string;
  #userId: string | null = null;

  constructor(matrix: MatrixGateway, storage: Storage, homeserver: string) {
    this.#matrix = matrix;
    this.#storage = storage;
    this.#key = `agent-room.matrix-authentication-attempt.v1:${homeserver}`;
  }

  async beginAuthentication(returnPath: string, mode: AuthenticationMode = 'interactive') {
    if (this.#userId === null) {
      return err(authenticationFailure('matrix.authentication_identity_missing'));
    }
    try {
      if (mode === 'automatic' && this.#storage.getItem(this.#key) === this.#userId) {
        return err(authenticationFailure('matrix.authentication_interrupted'));
      }
      // Set before opening either a browser SSO page or a native authentication window.
      this.#storage.setItem(this.#key, this.#userId);
    } catch {
      return err(storageFailure());
    }
    return await this.#matrix.beginAuthentication(returnPath);
  }

  async restore(expectedUserId: string) {
    this.#userId = expectedUserId;
    const restored = await this.#matrix.restore(expectedUserId);
    if (!restored.ok || restored.value.kind !== 'connected') return restored;
    const cleared = this.#clearAttempt();
    if (!cleared.ok) {
      this.#matrix.disconnect();
      return cleared;
    }
    return restored;
  }

  disconnect(): void {
    this.#matrix.disconnect();
  }

  async logout() {
    const loggedOut = await this.#matrix.logout();
    if (!loggedOut.ok) return loggedOut;
    this.#userId = null;
    return this.#clearAttempt();
  }

  #clearAttempt(): Result<void, SessionFailure> {
    try {
      this.#storage.removeItem(this.#key);
      return ok(undefined);
    } catch {
      return err(storageFailure());
    }
  }
}

function authenticationFailure(code: string): SessionFailure {
  return { boundary: 'matrix', code, offline: false, retryable: true };
}

function storageFailure(): SessionFailure {
  return {
    boundary: 'browser',
    code: 'browser.session_storage_unavailable',
    offline: false,
    retryable: true,
  };
}
