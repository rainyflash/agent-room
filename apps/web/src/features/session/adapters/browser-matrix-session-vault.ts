import {
  storedMatrixSessionSchema,
  type MatrixSessionVault,
  type StoredMatrixSession,
} from '@/features/session/domain/matrix-session-vault';
import type { SessionFailure } from '@/features/session/domain/session';
import { err, ok, type Result } from '@/shared/result';

const MATRIX_SESSION_KEY = 'agent-room.matrix-session.v1';

export class BrowserMatrixSessionVault implements MatrixSessionVault {
  constructor(private readonly storage: Storage) {}

  load(): Promise<Result<StoredMatrixSession | null, SessionFailure>> {
    return this.access(() => {
      const serialized = this.storage.getItem(MATRIX_SESSION_KEY);
      if (serialized === null) return null;
      const parsed = storedMatrixSessionSchema.safeParse(JSON.parse(serialized) as unknown);
      if (!parsed.success) {
        this.storage.removeItem(MATRIX_SESSION_KEY);
        return null;
      }
      return parsed.data;
    });
  }

  save(session: StoredMatrixSession): Promise<Result<void, SessionFailure>> {
    return this.access(() => {
      this.storage.setItem(MATRIX_SESSION_KEY, JSON.stringify(session));
    });
  }

  clear(): Promise<Result<void, SessionFailure>> {
    return this.access(() => {
      this.storage.removeItem(MATRIX_SESSION_KEY);
    });
  }

  private access<TValue>(operation: () => TValue): Promise<Result<TValue, SessionFailure>> {
    try {
      return Promise.resolve(ok(operation()));
    } catch {
      return Promise.resolve(
        err({
          boundary: 'browser',
          code: 'browser.session_storage_unavailable',
          offline: false,
          retryable: true,
        }),
      );
    }
  }
}
