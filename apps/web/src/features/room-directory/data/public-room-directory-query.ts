import { queryOptions, useQuery } from '@tanstack/react-query';

import type { PublicRoomDirectoryGateway } from '@/features/room-directory/domain/public-room-directory';

export const publicRoomDirectoryQueryKey = ['control-plane', 'public-room-directory'] as const;

export function publicRoomDirectoryQueryOptions(gateway: PublicRoomDirectoryGateway) {
  return queryOptions({
    networkMode: 'always',
    queryFn: async () => await gateway.list(),
    queryKey: publicRoomDirectoryQueryKey,
    retry: false,
    staleTime: 3_000,
  });
}

export function usePublicRoomDirectory(gateway: PublicRoomDirectoryGateway) {
  return useQuery(publicRoomDirectoryQueryOptions(gateway));
}
