import { queryOptions, useQuery } from '@tanstack/react-query';

import { privateRoomListQueryKey } from '@/features/private-rooms/data/private-room-queries';
import type { ProjectRoomGateway } from '@/features/private-rooms/domain/project-rooms';
import { err } from '@/shared/result';

/** 测试和夹具里没有这项服务时不去问；查询本来也不会跑。 */
const unavailable = { code: 'project_room.unavailable', retryable: false } as const;

/** 等着的认领，设置开着时隔一会儿再问一次：后台核对上了，这里跟着换成“绑上了”。 */
const PENDING_CLAIM_POLL_MS = 30_000;

/**
 * 这间房绑着哪个仓库（只有房主问得到）。挂在房间列表的键下面：改回私密、关掉都会解绑，列表刷新时这份
 * 跟着重问。
 */
export const roomRepositoryQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'repository', catalogId] as const;

/**
 * 房间名旁边的勾读的那一份（谁登录了都问得到）。挂在房主那一份的键下面：设置里绑上、解绑以后让房主那一份
 * 重问时，勾也跟着重问。
 */
export const roomRepositoryMarkQueryKey = (catalogId: string) =>
  [...roomRepositoryQueryKey(catalogId), 'mark'] as const;

export function useRoomRepository(
  gateway: ProjectRoomGateway | undefined,
  catalogId: string,
  enabled = true,
) {
  return useQuery(
    queryOptions({
      enabled: enabled && gateway !== undefined,
      queryKey: roomRepositoryQueryKey(catalogId),
      queryFn: async () =>
        gateway === undefined ? err(unavailable) : await gateway.binding(catalogId),
      networkMode: 'always',
      retry: false,
      staleTime: 30_000,
    }),
  );
}

/** 房间名旁边的勾：房间里的人都看得到，只在公开着的房间里问。 */
export function useRoomRepositoryMark(
  gateway: ProjectRoomGateway | undefined,
  catalogId: string,
  enabled: boolean,
) {
  return useQuery(
    queryOptions({
      enabled: enabled && gateway !== undefined,
      queryKey: roomRepositoryMarkQueryKey(catalogId),
      queryFn: async () =>
        gateway === undefined ? err(unavailable) : await gateway.roomRepository(catalogId),
      networkMode: 'always',
      retry: false,
      staleTime: 60_000,
    }),
  );
}

/** 自己在等的认领：设置里那一组拿它找这间房在等的那一个，所以每次打开都重问。 */
export const pendingClaimsQueryKey = ['control-plane', 'project-room-claim-list'] as const;

export function usePendingClaims(gateway: ProjectRoomGateway | undefined) {
  return useQuery(
    queryOptions({
      enabled: gateway !== undefined,
      queryKey: pendingClaimsQueryKey,
      queryFn: async () => (gateway === undefined ? err(unavailable) : await gateway.claims()),
      networkMode: 'always',
      retry: false,
      staleTime: 0,
    }),
  );
}

export const repositoryClaimQueryKey = (claimId: string) =>
  ['control-plane', 'project-room-claims', claimId] as const;

export function useRepositoryClaim(
  gateway: ProjectRoomGateway | undefined,
  claimId: string | null,
) {
  return useQuery(
    queryOptions({
      enabled: gateway !== undefined && claimId !== null,
      queryKey: repositoryClaimQueryKey(claimId ?? 'none'),
      queryFn: async () =>
        gateway === undefined || claimId === null
          ? err(unavailable)
          : await gateway.inspectClaim(claimId),
      networkMode: 'always',
      retry: false,
      staleTime: 5_000,
      refetchInterval: (query) => {
        const data = query.state.data;
        return data?.ok === true && data.value.status === 'pending' ? PENDING_CLAIM_POLL_MS : false;
      },
    }),
  );
}
