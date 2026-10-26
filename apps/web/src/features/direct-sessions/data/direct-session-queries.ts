import { queryOptions, useQuery } from '@tanstack/react-query';

import type { DirectSessionGateway } from '@/features/direct-sessions/domain/direct-session';

export const directSessionListQueryKey = ['control-plane', 'direct-sessions'] as const;
/** 设置里“屏蔽的 Agent”；在房间里屏蔽、解除屏蔽以后也要重读。 */
export const blockedAgentListQueryKey = ['control-plane', 'direct-contacts', 'blocks'] as const;

export function directSessionListQueryOptions(gateway: DirectSessionGateway, enabled = true) {
  return queryOptions({
    enabled,
    queryKey: directSessionListQueryKey,
    queryFn: async () => await gateway.list(),
    networkMode: 'always',
    retry: false,
    staleTime: 3_000,
  });
}

export function useDirectSessionList(gateway: DirectSessionGateway, enabled = true) {
  return useQuery(directSessionListQueryOptions(gateway, enabled));
}
