import { queryOptions, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect } from 'react';

import type { MatrixSecurityGateway } from '@/features/security/domain/matrix-security';

export const matrixSecurityQueryKey = ['matrix', 'security'] as const;

export function matrixSecurityQueryOptions(gateway: MatrixSecurityGateway, roomId?: string) {
  return queryOptions({
    queryKey: [...matrixSecurityQueryKey, roomId ?? 'account'] as const,
    queryFn: async () => await gateway.inspect(roomId === undefined ? {} : { roomId }),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export function useMatrixSecurity(gateway: MatrixSecurityGateway, roomId?: string) {
  const queryClient = useQueryClient();
  const query = useQuery(matrixSecurityQueryOptions(gateway, roomId));

  useEffect(
    () =>
      gateway.subscribe(() => {
        void queryClient.invalidateQueries({ queryKey: matrixSecurityQueryKey });
      }),
    [gateway, queryClient],
  );

  return query;
}
