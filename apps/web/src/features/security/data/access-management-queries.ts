import { queryOptions, useQuery } from '@tanstack/react-query';

import type { AccessManagementGateway } from '@/features/security/domain/access-management';

export const productDeviceQueryKey = ['control-plane', 'product-devices'] as const;
export const agentInstanceQueryKey = ['control-plane', 'agent-instances'] as const;

export function productDeviceQueryOptions(gateway: AccessManagementGateway) {
  return queryOptions({
    queryKey: productDeviceQueryKey,
    queryFn: async () => await gateway.listProductDevices(),
    networkMode: 'always',
    retry: false,
    staleTime: 3_000,
  });
}

export function agentInstanceQueryOptions(gateway: AccessManagementGateway) {
  return queryOptions({
    queryKey: agentInstanceQueryKey,
    queryFn: async () => await gateway.listAgentInstances(),
    networkMode: 'always',
    retry: false,
    staleTime: 3_000,
  });
}

export function useProductDevices(gateway: AccessManagementGateway) {
  return useQuery(productDeviceQueryOptions(gateway));
}

export function useAgentInstances(gateway: AccessManagementGateway) {
  return useQuery(agentInstanceQueryOptions(gateway));
}
