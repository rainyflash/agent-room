import { useInfiniteQuery, useQuery } from '@tanstack/react-query';

import {
  hasPendingRequest,
  JOIN_REQUEST_POLL_MS,
  type ExploreFilter,
  type ExploreGateway,
  type ExplorePage,
  type ExploreFailure,
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
    // 有在等批准的申请时隔一会儿重读，批准了好马上带他进去（`useApprovedJoin`）。
    refetchInterval: (query) =>
      hasPendingRequest(
        (query.state.data?.pages ?? []).flatMap((page) => (page.ok ? page.value.rooms : [])),
      )
        ? JOIN_REQUEST_POLL_MS
        : false,
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
    refetchInterval: (query) =>
      query.state.data?.ok === true && hasPendingRequest([query.state.data.value])
        ? JOIN_REQUEST_POLL_MS
        : false,
    retry: false,
    staleTime: 30_000,
  });
}
