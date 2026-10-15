import { queryOptions, useQueries, useQuery } from '@tanstack/react-query';

import type { AgentDirectoryGateway } from '@/features/workspace/domain/agent-directory';

export const agentRoomsQueryKey = ['control-plane', 'agent-rooms'] as const;

export function agentRoomsQueryOptions(gateway: AgentDirectoryGateway, agentId: string) {
  return queryOptions({
    networkMode: 'always',
    queryFn: async () => await gateway.listAgentRooms(agentId),
    queryKey: [...agentRoomsQueryKey, agentId],
    retry: false,
    staleTime: 5_000,
  });
}

export function useAgentRooms(gateway: AgentDirectoryGateway, agentId: string) {
  return useQuery(agentRoomsQueryOptions(gateway, agentId));
}

/** 每个 Agent 在几个房间，卡片上用；还没问到、没问成的不在里面。 */
export function useAgentRoomCounts(
  gateway: AgentDirectoryGateway,
  agentIds: readonly string[],
): ReadonlyMap<string, number> {
  return useQueries({
    queries: agentIds.map((agentId) => agentRoomsQueryOptions(gateway, agentId)),
    combine: (results) => {
      const counts = new Map<string, number>();
      results.forEach((result, index) => {
        const agentId = agentIds[index];
        if (agentId !== undefined && result.data?.ok === true) {
          counts.set(agentId, result.data.value.length);
        }
      });
      return counts;
    },
  });
}
