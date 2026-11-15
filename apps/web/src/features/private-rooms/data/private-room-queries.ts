import { queryOptions, useQueries, useQuery } from '@tanstack/react-query';
import { useMemo } from 'react';

import { err, ok } from '@/shared/result';

import type { PrivateRoomGateway } from '@/features/private-rooms/domain/private-room';
import type { PrivateRoomInviteGateway } from '@/features/private-rooms/domain/private-room-invite-link';
import type { PrivateRoomListingGateway } from '@/features/private-rooms/domain/private-room-listing';
import type { PublicRoomJoinGateway } from '@/features/private-rooms/domain/public-room-join';
import type { RoomRoleChangeGateway } from '@/features/private-rooms/domain/room-role-changes';

/** 测试和夹具里没有这项服务时不去问；查询本来也不会跑。 */
const unavailable = { code: 'private_room.unavailable', retryable: false } as const;

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

export function usePrivateRoomList(
  gateway: PrivateRoomGateway,
  accountId?: string,
  enabled = true,
) {
  return useQuery({ ...privateRoomListQueryOptions(gateway, accountId), enabled });
}

export const privateRoomQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'room', catalogId] as const;

/**
 * 单独问一间房（`GET /private-rooms/{c}`）：子房间不在房间列表里（旧版网页不认得它们），打开子房间时
 * 这样问。你不在里面、受邀也不是的回“找不到”。挂在房间列表的键下面，列表刷新时跟着重问。
 */
export function usePrivateRoom(gateway: PrivateRoomGateway, catalogId: string, enabled = true) {
  return useQuery({
    enabled,
    queryKey: privateRoomQueryKey(catalogId),
    queryFn: async () => await gateway.inspect(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

/**
 * 房间列表加上眼前这一间：列表里没有它（子房间）时单独问一次，问到了接在列表后面，问不到（公开大厅、
 * 你不在里面）就只是列表。`data` 和房间列表是同一个形状，单独问的那次回来以前没有。`lookUp` 为 false
 * 时不单独问（已经知道它不是子房间，免得公开大厅白问一次）。
 */
export function usePrivateRoomListWith(
  gateway: PrivateRoomGateway,
  accountId: string | undefined,
  catalogId: string,
  enabled = true,
  lookUp = true,
) {
  const list = usePrivateRoomList(gateway, accountId, enabled);
  const missing =
    lookUp &&
    list.data?.ok === true &&
    !list.data.value.some((room) => room.catalogId === catalogId);
  const single = usePrivateRoom(gateway, catalogId, enabled && missing);
  const data = useMemo(() => {
    if (list.data?.ok !== true || !missing) return list.data;
    if (single.data === undefined) return single.isError ? list.data : undefined;
    return single.data.ok ? ok([...list.data.value, single.data.value]) : list.data;
  }, [list.data, missing, single.data, single.isError]);
  return {
    data,
    isError: list.isError || (missing && single.isError),
    isPending: list.isPending || (missing && single.data === undefined && !single.isError),
    refetch: async () => {
      await Promise.all([list.refetch(), ...(missing ? [single.refetch()] : [])]);
    },
  };
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

export const privateRoomPeopleQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'people', catalogId] as const;

/**
 * 成员的昵称；只有已加入的成员问得到。挂在房间列表的键下面：邀请、移出、封禁以后列表刷新，名字跟着重问，
 * 新来的人马上有名字。
 */
export function privateRoomPeopleQueryOptions(gateway: PrivateRoomGateway, catalogId: string) {
  return queryOptions({
    queryKey: privateRoomPeopleQueryKey(catalogId),
    queryFn: async () => await gateway.people(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 30_000,
  });
}

export function usePrivateRoomPeople(
  gateway: PrivateRoomGateway,
  catalogId: string,
  enabled = true,
) {
  return useQuery({ ...privateRoomPeopleQueryOptions(gateway, catalogId), enabled });
}

export const roomRoleChangesQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'role-changes', catalogId] as const;

/**
 * 身份变动记录（`specs/redesign/room-roles.md` 的“记录”）；只有房主和管理员问得到。挂在房间列表的键
 * 下面：改了谁的身份以后列表刷新，开着的记录跟着重读，新的那一条马上在最上面。
 */
export function useRoomRoleChanges(
  gateway: RoomRoleChangeGateway | undefined,
  catalogId: string,
  enabled = true,
) {
  return useQuery({
    enabled: enabled && gateway !== undefined,
    queryKey: roomRoleChangesQueryKey(catalogId),
    queryFn: async () =>
      gateway === undefined ? err(unavailable) : await gateway.roleChanges(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
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

export const privateRoomInviteLinkQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'invite-link', catalogId] as const;

/**
 * 这个房间现在的邀请链接（`specs/room-people/design.md`）：没有时回 `invite_link.not_found`，
 * 看不到时 `invite_link.not_revealable`。生成、停用的地方直接改这份缓存，不用再问一次。
 */
export function usePrivateRoomInviteLink(gateway: PrivateRoomInviteGateway, catalogId: string) {
  return useQuery({
    queryKey: privateRoomInviteLinkQueryKey(catalogId),
    queryFn: async () => await gateway.inviteLink(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export const privateRoomListingQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'listing', catalogId] as const;

/**
 * 房间的公开设置（`specs/public-rooms/design.md`）；已加入的成员都问得到。挂在房间列表的键下面：
 * 改了设置以后列表刷新（改回私密会打开加密），这份跟着重问。改设置、换码的地方直接写进这份缓存。
 */
function privateRoomListingQueryOptions(
  gateway: PrivateRoomListingGateway | undefined,
  catalogId: string,
  enabled = true,
) {
  return queryOptions({
    enabled: enabled && gateway !== undefined,
    queryKey: privateRoomListingQueryKey(catalogId),
    queryFn: async () =>
      gateway === undefined ? err(unavailable) : await gateway.listing(catalogId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}

export function usePrivateRoomListing(
  gateway: PrivateRoomListingGateway | undefined,
  catalogId: string,
  enabled = true,
) {
  return useQuery(privateRoomListingQueryOptions(gateway, catalogId, enabled));
}

/** 这几间房里哪些公开着；没问到、问不成的当私密的。只给你已加入的房间用：别的问了服务器也只回 403。 */
export function useListedPrivateRooms(
  gateway: PrivateRoomListingGateway | undefined,
  catalogIds: readonly string[],
): ReadonlySet<string> {
  return useQueries({
    queries: catalogIds.map((catalogId) => privateRoomListingQueryOptions(gateway, catalogId)),
    combine: (results) =>
      new Set(
        catalogIds.filter((_, index) => {
          const listing = results[index]?.data;
          return listing?.ok === true && listing.value.listed;
        }),
      ),
  });
}

export const privateRoomJoinRequestsQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'join-requests', catalogId] as const;

/**
 * 在等的申请（`specs/public-rooms/design.md`“加入方式”）；只有房主和管理员问得到。`intervalMs` 给了
 * 就按它定时再问（房间页 10 秒，齿轮上的点跟着它），页面在后台时不问，回到前台时马上问一次。
 */
export function usePrivateRoomJoinRequests(
  gateway: PublicRoomJoinGateway | undefined,
  catalogId: string,
  { enabled = true, intervalMs }: { readonly enabled?: boolean; readonly intervalMs?: number } = {},
) {
  return useQuery({
    enabled: enabled && gateway !== undefined,
    queryKey: privateRoomJoinRequestsQueryKey(catalogId),
    queryFn: async () =>
      gateway === undefined ? err(unavailable) : await gateway.joinRequests(catalogId),
    networkMode: 'always',
    refetchInterval: intervalMs ?? false,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: 'always',
    retry: false,
    staleTime: 1_000,
  });
}
