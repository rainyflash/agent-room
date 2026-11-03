import { useInfiniteQuery, useQuery } from '@tanstack/react-query';

import type {
  ExploreFilter,
  ExploreGateway,
  ExplorePage,
  ExploreFailure,
} from '@/features/explore/domain/explore';
import { err, type Result } from '@/shared/result';

const unavailable: ExploreFailure = { code: 'public_room.unavailable', retryable: false };

/**
 * 发现页的键带上是谁在看：登录、退出以后“你在不在里面”跟着变。键不挂在登录范围里，没登录的人也用。
 */
export const exploreQueryKey = ['public-rooms'] as const;

export function useExploreRooms(
  gateway: ExploreGateway | undefined,
  filter: ExploreFilter,
  viewer: string | null,
  enabled: boolean,
) {
  return useInfiniteQuery({
    enabled: enabled && gateway !== undefined,
    getNextPageParam: (last: Result<ExplorePage, ExploreFailure>) =>
      last.ok ? (last.value.nextCursor ?? undefined) : undefined,
    initialPageParam: null as string | null,
    networkMode: 'always',
    queryFn: async ({ pageParam }) =>
      gateway === undefined ? err(unavailable) : await gateway.browse(filter, pageParam),
    queryKey: [...exploreQueryKey, 'browse', viewer, filter] as const,
    retry: false,
    staleTime: 30_000,
  });
}

export function useExploreRoom(
  gateway: ExploreGateway | undefined,
  shareCode: string | null,
  viewer: string | null,
  enabled: boolean,
) {
  return useQuery({
    enabled: enabled && gateway !== undefined && shareCode !== null,
    networkMode: 'always',
    queryFn: async () =>
      gateway === undefined || shareCode === null
        ? err(unavailable)
        : await gateway.detail(shareCode),
    queryKey: [...exploreQueryKey, 'detail', viewer, shareCode] as const,
    retry: false,
    staleTime: 30_000,
  });
}
