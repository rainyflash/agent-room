import { queryOptions, useQuery } from '@tanstack/react-query';

import type { ReadinessGateway } from '@/features/health/domain/readiness';

export function readinessQueryOptions(gateway: ReadinessGateway) {
  return queryOptions({
    queryKey: ['control-plane', 'readiness'] as const,
    queryFn: async () => await gateway.readReadiness(),
    networkMode: 'always',
    refetchInterval: (query) => {
      const report = query.state.data;
      return report?.ok === true && report.value.status === 'ready' ? 15_000 : 5_000;
    },
    retry: false,
    staleTime: 4_000,
  });
}

export function useReadiness(gateway: ReadinessGateway) {
  return useQuery(readinessQueryOptions(gateway));
}
