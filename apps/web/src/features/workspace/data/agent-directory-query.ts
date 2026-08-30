import { queryOptions, useQuery } from '@tanstack/react-query';

import type { AgentDirectoryGateway } from '@/features/workspace/domain/agent-directory';

export const ownedAgentQueryKey = ['control-plane', 'owned-agents'] as const;

export function ownedAgentQueryOptions(gateway: AgentDirectoryGateway) {
  return queryOptions({
    networkMode: 'always',
    queryFn: async () => await gateway.listOwnedAgents(),
    queryKey: ownedAgentQueryKey,
    retry: false,
    staleTime: 3_000,
  });
}

export function useOwnedAgents(gateway: AgentDirectoryGateway) {
  return useQuery(ownedAgentQueryOptions(gateway));
}
