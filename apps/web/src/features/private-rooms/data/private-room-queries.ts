import { queryOptions, useQueries, useQuery } from '@tanstack/react-query';

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

export const privateRoomOriginQueryKey = (catalogId: string) =>
  ['control-plane', 'private-room-origin', catalogId] as const;

/**
 * 房间是谁建的。建好以后不会变：问到了就一直用，没问到的下次用到时再问。不挂在房间列表的键下面，
 * 房间列表刷新时不跟着重问。
 */
export function privateRoomOriginQueryOptions(gateway: PrivateRoomGateway, catalogId: string) {
  return queryOptions({
    queryKey: privateRoomOriginQueryKey(catalogId),
    queryFn: async () => await gateway.origin(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: (query) => (query.state.data?.ok === true ? Infinity : 0),
  });
}

export function usePrivateRoomOrigin(
  gateway: PrivateRoomGateway,
  catalogId: string,
  enabled = true,
) {
  return useQuery({ ...privateRoomOriginQueryOptions(gateway, catalogId), enabled });
}

/** 这些房间里哪些是网络 Agent 建的、服务器读得到；还没问到的不算。 */
export function useServerReadableRooms(
  gateway: PrivateRoomGateway,
  catalogIds: readonly string[],
): ReadonlySet<string> {
  return useQueries({
    queries: catalogIds.map((catalogId) => privateRoomOriginQueryOptions(gateway, catalogId)),
    combine: (results) =>
      new Set(
        catalogIds.filter((_, index) => {
          const origin = results[index]?.data;
          return origin?.ok === true && origin.value.ownerIsNetworkAgent;
        }),
      ),
  });
}

export const privateRoomAgentAccessQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'agent-access', catalogId] as const;

/** 口令状态与凭口令进来的 Agent；只有能管理房间的人查得到。 */
export function usePrivateRoomAgentAccess(
  gateway: PrivateRoomGateway,
  catalogId: string,
  enabled = true,
) {
  return useQuery({
    enabled,
    queryKey: privateRoomAgentAccessQueryKey(catalogId),
    queryFn: async () => await gateway.agentAccess(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

/** 同一个房间每换一次口令就是另一份缓存：生成时间不同，旧口令不会被当成现在的。 */
export const privateRoomJoinCodeQueryKey = (catalogId: string, createdAtUnixMs: number) =>
  [...privateRoomListQueryKey, 'join-code', catalogId, createdAtUnixMs] as const;

/**
 * 管理者再看口令（`specs/network-agents/code-first.md`）。`createdAtUnixMs` 是口令状态里的生成时间，
 * 没有口令时传 null，不去查。生成口令的回答里带着口令，生成的地方直接写进这份缓存，不用再问一次。
 */
export function usePrivateRoomJoinCode(
  gateway: PrivateRoomGateway,
  catalogId: string,
  createdAtUnixMs: number | null,
) {
  return useQuery({
    enabled: createdAtUnixMs !== null,
    queryKey: privateRoomJoinCodeQueryKey(catalogId, createdAtUnixMs ?? 0),
    queryFn: async () => await gateway.revealJoinCode(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export const privateRoomAgentKnocksQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'agent-knocks', catalogId] as const;

/**
 * 在敲门的网络 Agent；只有能管理房间的人查得到。`intervalMs` 给了就按它定时再问一次，
 * 页面在后台时不问（接入对话框开着时 3 秒，房间页 10 秒），回到前台时马上问一次。
 */
export function usePrivateRoomAgentKnocks(
  gateway: PrivateRoomGateway,
  catalogId: string,
  { enabled = true, intervalMs }: { readonly enabled?: boolean; readonly intervalMs?: number } = {},
) {
  return useQuery({
    enabled,
    queryKey: privateRoomAgentKnocksQueryKey(catalogId),
    queryFn: async () => await gateway.agentKnocks(catalogId),
    networkMode: 'always',
    refetchInterval: intervalMs ?? false,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: 'always',
    retry: false,
    staleTime: 1_000,
  });
}
