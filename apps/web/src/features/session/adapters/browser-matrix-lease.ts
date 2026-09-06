import type { SessionFailure } from '../domain/session';
import { err, ok, type Result } from '@/shared/result';

export type MatrixCryptoLease = { release(): void };

/** 同一浏览器设备的 Rust Crypto 数据库只能由一个窗口运行。关窗自动释放。 */
export function acquireMatrixCryptoLease(
  name: string,
  locks: LockManager | undefined,
): Promise<Result<MatrixCryptoLease, SessionFailure>> {
  const unavailable: SessionFailure = {
    boundary: 'browser',
    code: 'browser.session_lock_unavailable',
    offline: false,
    retryable: true,
  };
  if (locks === undefined) return Promise.resolve(err(unavailable));
  const inUse: SessionFailure = {
    boundary: 'matrix',
    code: 'matrix.session_in_use',
    offline: false,
    retryable: true,
  };
  // 关窗后的锁释放是异步的，给正在关闭的窗口一小段交接时间。
  const signal = AbortSignal.timeout(1_000);
  return new Promise((resolve) => {
    void Promise.resolve()
      .then(() =>
        locks.request(name, { signal }, async (lock) => {
          if (lock === null) {
            resolve(err(inUse));
            return;
          }
          await new Promise<void>((release) => {
            resolve(ok({ release }));
          });
        }),
      )
      .catch(() => {
        resolve(err(signal.aborted ? inUse : unavailable));
      });
  });
}
