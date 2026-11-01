import { useQuery } from '@tanstack/react-query';

import type { AgentDirectoryGateway } from '@/features/workspace/domain/agent-directory';

export const agentInstanceActivityQueryKey = ['control-plane', 'agent-instance-activity'] as const;

/**
 * 每个还在用的连接的设备上次用的时间。服务器要替每个连接问一遍聊天服务器，所以只在可能有
 * 30 天没上线的 Agent 时才问（`enabled`）。
 */
export function useAgentInstanceActivity(gateway: AgentDirectoryGateway, enabled: boolean) {
  return useQuery({
    enabled,
    networkMode: 'always',
    queryFn: async () => await gateway.listInstanceActivity(),
    queryKey: agentInstanceActivityQueryKey,
    retry: false,
    staleTime: 60_000,
  });
}
