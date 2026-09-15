import { queryOptions, useQuery } from '@tanstack/react-query';

import type { PrivateRoomGateway } from '@/features/private-rooms/domain/private-room';

export const privateRoomListQueryKey = ['control-plane', 'private-rooms'] as const;

export function privateRoomListQueryOptions(gateway: PrivateRoomGateway, accountId?: string) {
  return queryOptions({
    queryKey: [...privateRoomListQueryKey, accountId ?? 'session'],
    queryFn: async () => await gateway.list(),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export function usePrivateRoomList(gateway: PrivateRoomGateway, accountId?: string) {
  return useQuery(privateRoomListQueryOptions(gateway, accountId));
}
