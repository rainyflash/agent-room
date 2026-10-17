import type { MatrixPresenceObservation } from '@agent-room/protocol';
import { useEffect, useState } from 'react';

import type { AgentPresenceSource } from '@/features/lobby/adapters/matrix-presence-tracker';

/**
 * 隔多久再读一次。在线状态随同步进来，来源只在问到人的时候通知，同步里变了的要自己再读；
 * 读的是内存里的记录，不发请求。
 */
const PRESENCE_REFRESH_MS = 5_000;

const NONE: ReadonlyMap<string, MatrixPresenceObservation> = new Map();

/** 这些 Agent 此刻的 Matrix 在线状态，按 Matrix 用户 ID 记；问不到的不在里面。没有来源时是空的。 */
export function useAgentPresence(
  source: AgentPresenceSource | undefined,
  matrixUserIds: readonly string[],
): ReadonlyMap<string, MatrixPresenceObservation> {
  const key = [...new Set(matrixUserIds)].sort().join('\n');
  const [snapshot, setSnapshot] = useState(NONE);
  useEffect(() => {
    if (source === undefined || key === '') {
      setSnapshot(NONE);
      return undefined;
    }
    const userIds = key.split('\n');
    const read = (): void => {
      const next = source.observe(userIds);
      setSnapshot((previous) => (sameObservations(previous, next) ? previous : next));
    };
    read();
    const unsubscribe = source.subscribe(read);
    const timer = window.setInterval(read, PRESENCE_REFRESH_MS);
    return () => {
      unsubscribe();
      window.clearInterval(timer);
    };
  }, [source, key]);
  return snapshot;
}

function sameObservations(
  left: ReadonlyMap<string, MatrixPresenceObservation>,
  right: ReadonlyMap<string, MatrixPresenceObservation>,
): boolean {
  if (left.size !== right.size) return false;
  for (const [userId, observation] of right) {
    const previous = left.get(userId);
    if (
      previous?.state !== observation.state ||
      previous.offlineSeenAtUnixMs !== observation.offlineSeenAtUnixMs ||
      previous.lastActiveAtUnixMs !== observation.lastActiveAtUnixMs
    )
      return false;
  }
  return true;
}
