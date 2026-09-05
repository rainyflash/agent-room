import {
  storedMatrixSessionSchema,
  type MatrixSessionVault,
  type StoredMatrixSession,
} from './matrix-session-vault';
import type { SessionFailure } from './session';
import { err, ok, type Result } from '@/shared/result';

/** 串行化凭据轮换与落盘；网络连接本身不负责存储生命周期。 */
export class MatrixSessionRepository {
  #epoch = 0;
  #queue: Promise<void> = Promise.resolve();
  #pending: StoredMatrixSession | null = null;
  #failure: SessionFailure | null = null;

  constructor(private readonly vault: MatrixSessionVault) {}

  get epoch(): number {
    return this.#epoch;
  }

  get failure(): SessionFailure | null {
    return this.#failure;
  }

  beginSession(): number {
    ++this.#epoch;
    this.#pending = null;
    this.#failure = null;
    return this.#epoch;
  }

  load(): Promise<Result<StoredMatrixSession | null, SessionFailure>> {
    return this.serial(async () => {
      // 刷新后的令牌可能已经使旧令牌失效；落盘失败时先重试当前进程保存的最新值。
      if (this.#pending !== null) {
        const pending = this.#pending;
        const saved = await this.persist(pending, this.#epoch);
        return saved.ok ? ok(pending) : saved;
      }
      const loaded = await this.vault.load();
      this.#failure = loaded.ok ? null : loaded.error;
      return loaded;
    });
  }

  save(session: StoredMatrixSession, epoch: number): Promise<Result<void, SessionFailure>> {
    return this.serial(() => this.persist(session, epoch));
  }

  rotate(
    epoch: number,
    currentConnection: () => boolean,
    refresh: () => Promise<StoredMatrixSession>,
  ): Promise<Result<StoredMatrixSession, SessionFailure>> {
    return this.serial(async () => {
      if (epoch !== this.#epoch || !currentConnection()) return err(supersededMatrixSession());
      const session = await refresh();
      const saved = await this.persist(session, epoch);
      return saved.ok ? ok(session) : saved;
    });
  }

  clear(): Promise<Result<void, SessionFailure>> {
    this.beginSession();
    return this.serial(async () => {
      const result = await this.vault.clear();
      this.#failure = result.ok ? null : result.error;
      return result;
    });
  }

  private async persist(
    session: StoredMatrixSession,
    epoch: number,
  ): Promise<Result<void, SessionFailure>> {
    if (epoch !== this.#epoch) return err(supersededMatrixSession());
    const parsed = storedMatrixSessionSchema.safeParse(session);
    if (!parsed.success) {
      this.#failure = {
        boundary: 'matrix',
        code: 'matrix.invalid_session',
        offline: false,
        retryable: true,
      };
      return err(this.#failure);
    }
    this.#pending = parsed.data;
    const result = await this.vault.save(parsed.data);
    if (epoch !== this.#epoch) return err(supersededMatrixSession());
    this.#failure = result.ok ? null : result.error;
    if (result.ok) this.#pending = null;
    return result;
  }

  private serial<TValue>(operation: () => Promise<TValue>): Promise<TValue> {
    const result = this.#queue.then(operation);
    // 仅恢复队列本身；原始 Promise 仍向调用者传播失败。
    this.#queue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }
}

export function supersededMatrixSession(): SessionFailure {
  return { boundary: 'matrix', code: 'matrix.session_superseded', offline: false, retryable: true };
}
