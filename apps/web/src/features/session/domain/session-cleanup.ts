import type { SessionDependencies, SessionFailure } from './session';
import { err, ok, type Result } from '@/shared/result';

/** 各凭据边界独立清理；一个适配器异常不能阻断另一个边界的注销。 */
export async function cleanupSession(
  dependencies: SessionDependencies,
  controlSessionExpired = false,
): Promise<Result<void, SessionFailure>> {
  const boundaries = ['matrix', 'control-plane'] as const;
  const results = await Promise.allSettled([
    Promise.resolve().then(() => dependencies.matrix.logout()),
    Promise.resolve().then(() =>
      controlSessionExpired ? ok(undefined) : dependencies.controlPlane.logout(),
    ),
  ]);
  for (const [index, result] of results.entries()) {
    if (result.status === 'rejected') {
      return err({
        boundary: boundaries[index] ?? 'browser',
        code: 'session.cleanup_failed',
        offline: false,
        retryable: true,
      });
    }
    if (!result.value.ok) return result.value;
  }
  return ok(undefined);
}
