import { useQuery } from '@tanstack/react-query';
import type { DesktopRuntimeGateway } from '../domain/desktop-runtime';
import { err } from '@/shared/result';

export const receptionQueryKey = (principalId: string | null) => ['desktop-reception', principalId];

/** UI diagnostics are shared across panels; they never call or wake the model. */
export function useReceptionQueries(gateway: DesktopRuntimeGateway, principalId: string | null) {
  const available = gateway.isAvailable() && principalId !== null;
  const queryKey = receptionQueryKey(principalId);
  const receivers = useQuery({
    queryKey: [...queryKey, 'receivers'],
    queryFn: () =>
      gateway.listReceivers?.() ?? err({ code: 'receiver.unavailable', retryable: false }),
    enabled: available,
    refetchInterval: 3000,
    staleTime: 2000,
  });
  const sessions = useQuery({
    queryKey: [...queryKey, 'sessions'],
    queryFn: () =>
      gateway.readHostSessions?.() ?? err({ code: 'receiver.unavailable', retryable: false }),
    enabled: available,
    refetchInterval: 5000,
    staleTime: 2000,
  });
  return { available, queryKey, receivers, sessions };
}
