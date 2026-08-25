import { queryOptions, useQuery } from '@tanstack/react-query';

import type { DirectSessionGateway } from '@/features/direct-sessions/domain/direct-session';

export const directSessionListQueryKey = ['control-plane', 'direct-sessions'] as const;

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
