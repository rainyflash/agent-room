import type { Result } from '@/shared/result';

import type {
  DesktopRuntimeFailure,
  ReleaseUpdateChannel,
  ReleaseUpdateCheck,
  ReleaseUpdateStatus,
} from './desktop-runtime';

/** 通道清单签名后 7 天过期：两次发版隔了一周以上就查不到，下次发版以后就好，不算故障。 */
export const manifestExpiredCode = 'desktop.update.manifest_expired';
/** macOS 上应用在只读位置运行（没拖进“应用程序”文件夹），更新换不了自己。 */
export const appTranslocatedCode = 'desktop.update.app_translocated';

/**
 * 手动检查的结果先在网页这边记下，记法和原生层一样：没查成时留着上一次查成的结果。
 * 原生层随后也会发来同一次检查的结果。
 */
export function updateStatusAfterCheck(
  previous: ReleaseUpdateStatus | null,
  channel: ReleaseUpdateChannel,
  result: Result<ReleaseUpdateCheck, DesktopRuntimeFailure>,
  checkedAtUnixMs: number,
): ReleaseUpdateStatus {
  return result.ok
    ? { checkedAtUnixMs, channel, check: result.value, failure: null }
    : {
        checkedAtUnixMs,
        channel,
        check: previous?.check ?? null,
        failure: { code: result.error.code, retryable: result.error.retryable },
      };
}

/** 两份检查结果留时间新的：网页加载时读的快照和刚到的事件，谁先到说不准。 */
export function newerUpdateStatus(
  current: ReleaseUpdateStatus | null,
  next: ReleaseUpdateStatus | null,
): ReleaseUpdateStatus | null {
  if (next === null) return current;
  if (current === null) return next;
  return next.checkedAtUnixMs >= current.checkedAtUnixMs ? next : current;
}
