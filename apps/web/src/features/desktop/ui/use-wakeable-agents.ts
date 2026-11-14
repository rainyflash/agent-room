import { useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';

import { useAppServices } from '@/app/app-services';
import { err } from '@/shared/result';

import { wakeableAgentIds } from '../domain/wakeable-agents';
import { useReceptionEvidence } from './reception-evidence-context';
import { receptionOwnershipQueryKeys } from './reception-ownership-panel';

const none: ReadonlySet<string> = new Set();

/**
 * 这间房里此刻叫得醒的 Agent（开着后台回复、有电脑正替它收着）。只知道你自己的，所以 `enabled`
 * 传“这间房里有没有你的 Agent”：没有就不去问。
 *
 * 账户上的后台回复和 Agent 详情里那一块读的是同一份（同一个查询键），这里半分钟读一次。
 */
export function useWakeableAgentIds(
  roomId: string,
  principalId: string | null,
  enabled: boolean,
): ReadonlySet<string> {
  const { receptionOwnership: gateway } = useAppServices();
  const receivers = useReceptionEvidence();
  const records = useQuery({
    queryKey: receptionOwnershipQueryKeys.records(principalId),
    queryFn: () => gateway?.list() ?? err({ code: 'reception.unavailable', retryable: false }),
    enabled: enabled && gateway !== undefined && principalId !== null,
    refetchInterval: 30_000,
  });
  const receptions = records.data?.ok === true ? records.data.value.receptions : null;
  const readAt = records.dataUpdatedAt;
  return useMemo(() => {
    if (!enabled) return none;
    const ids = wakeableAgentIds(roomId, receptions ?? [], readAt, receivers);
    return ids.size === 0 ? none : ids;
  }, [enabled, readAt, receivers, receptions, roomId]);
}
