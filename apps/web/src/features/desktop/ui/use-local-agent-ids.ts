import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';

import { useAppServices } from '@/app/app-services';
import { err } from '@/shared/result';

/**
 * 这台电脑上接进来的 Agent（连接服务此刻记着的会话）。只有桌面端问得到；浏览器里、还没读到、
 * 读不出来时回 `null`，意思是“不知道”，不是“一个都没有”。
 *
 * `enabled` 传“现在要不要知道”：只有 Agent 表格的整页用它，不开着就不去问连接服务。
 */
export function useLocalAgentIds(enabled: boolean): ReadonlySet<string> | null {
  const { localRuntime } = useAppServices();
  const sessions = useQuery({
    queryKey: ['desktop', 'host-sessions', 'agent-ids'],
    queryFn: () =>
      localRuntime.readHostSessions?.() ??
      err({ code: 'desktop.hosts.diagnostics_unavailable', retryable: false }),
    enabled: enabled && localRuntime.isAvailable() && localRuntime.readHostSessions !== undefined,
    refetchInterval: 30_000,
  });
  const entries = sessions.data?.ok === true ? sessions.data.value : null;
  return useMemo(
    () =>
      entries === null ? null : new Set(entries.flatMap(({ session }) => session.agentId ?? [])),
    [entries],
  );
}
