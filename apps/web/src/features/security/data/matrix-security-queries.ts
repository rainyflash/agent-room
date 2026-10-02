import { queryOptions, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect } from 'react';

import type { MatrixSecurityGateway } from '@/features/security/domain/matrix-security';

export const matrixSecurityQueryKey = ['matrix', 'security'] as const;

export function matrixSecurityQueryOptions(gateway: MatrixSecurityGateway) {
  return queryOptions({
    queryKey: [...matrixSecurityQueryKey, 'account'] as const,
    queryFn: async () => await gateway.inspect(),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export function useMatrixSecurity(gateway: MatrixSecurityGateway) {
  const queryClient = useQueryClient();
  const query = useQuery(matrixSecurityQueryOptions(gateway));

  useEffect(
    () =>
      gateway.subscribe(() => {
        void queryClient.invalidateQueries({ queryKey: matrixSecurityQueryKey });
      }),
    [gateway, queryClient],
  );

  return query;
}
