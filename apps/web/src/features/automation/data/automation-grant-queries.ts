import { queryOptions, useQuery } from '@tanstack/react-query';

import type { AutomationGrantGateway } from '@/features/automation/domain/automation-grant';

export const automationGrantListQueryKey = ['control-plane', 'automation-grants'] as const;

export function automationGrantListQueryOptions(gateway: AutomationGrantGateway) {
  return queryOptions({
    queryFn: async () => await gateway.list(),
    queryKey: automationGrantListQueryKey,
    networkMode: 'always',
    refetchInterval: 15_000,
    retry: false,
    staleTime: 3_000,
  });
}

export function useAutomationGrantList(gateway: AutomationGrantGateway) {
  return useQuery(automationGrantListQueryOptions(gateway));
}
