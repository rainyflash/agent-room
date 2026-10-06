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
