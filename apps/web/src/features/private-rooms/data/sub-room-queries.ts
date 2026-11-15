import { useQuery } from '@tanstack/react-query';

import { err } from '@/shared/result';

import { privateRoomListQueryKey } from '@/features/private-rooms/data/private-room-queries';
import {
  managesSubRoom,
  type SubRoomGateway,
  type SubRoomRef,
} from '@/features/private-rooms/domain/sub-rooms';

/** 测试和夹具里没有这项服务时不去问；查询本来也不会跑。 */
const unavailable = { code: 'private_room.unavailable', retryable: false } as const;

/**
 * 子房间的查询都挂在房间列表的键下面：进出、批准、移出以后界面照旧刷新房间列表，名单、申请跟着重问。
 * 只刷新子房间的用这个前缀。
 */
export const subRoomsQueryKey = [...privateRoomListQueryKey, 'sub-rooms'] as const;

export const subRoomListQueryKey = (roomId: string, archived: boolean) =>
  [...subRoomsQueryKey, 'list', roomId, archived ? 'archived' : 'active'] as const;

type PollingOptions = { readonly enabled?: boolean; readonly intervalMs?: number };

/**
 * 一间房下面你看得到的子房间。`roomId` 是父房间或者它的任何一个子房间：子房间的页面这样问出它属于谁。
 * `intervalMs` 给了就按它定时再问，页面在后台时不问。`managingIntervalMs` 是名单上有你管得了的子房间时
 * 多久再问一次：左栏和齿轮上“有人在等批准”的点跟着它。
 */
export function useSubRooms(
  gateway: SubRoomGateway | undefined,
  roomId: string,
  {
    archived = false,
    enabled = true,
    intervalMs,
    managingIntervalMs,
  }: PollingOptions & { readonly archived?: boolean; readonly managingIntervalMs?: number } = {},
) {
  return useQuery({
    enabled: enabled && gateway !== undefined,
    queryKey: subRoomListQueryKey(roomId, archived),
    queryFn: async () =>
      gateway === undefined ? err(unavailable) : await gateway.subRooms(roomId, { archived }),
    networkMode: 'always',
    refetchInterval: (query) => {
      const data = query.state.data;
      const managing =
        managingIntervalMs !== undefined &&
        data?.ok === true &&
        data.value.subRooms.some(managesSubRoom);
      return managing ? managingIntervalMs : (intervalMs ?? false);
    },
    refetchIntervalInBackground: false,
    retry: false,
    staleTime: 5_000,
  });
}

export const subRoomJoinRequestsQueryKey = ({ parentId, subRoomId }: SubRoomRef) =>
  [...subRoomsQueryKey, 'join-requests', parentId, subRoomId] as const;

/** 一间子房间里在等的申请，先来的在前；只有管得了这一间的人问得到。 */
export function useSubRoomJoinRequests(
  gateway: SubRoomGateway | undefined,
  room: SubRoomRef,
  { enabled = true, intervalMs }: PollingOptions = {},
) {
  return useQuery({
    enabled: enabled && gateway !== undefined,
    queryKey: subRoomJoinRequestsQueryKey(room),
    queryFn: async () =>
      gateway === undefined ? err(unavailable) : await gateway.joinRequests(room),
    networkMode: 'always',
    refetchInterval: intervalMs ?? false,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: 'always',
    retry: false,
    staleTime: 1_000,
  });
}

export const subRoomPolicyQueryKey = (parentId: string) =>
  [...subRoomsQueryKey, 'policy', parentId] as const;

/** 房间设置里“子房间”一节的三项；父房间里已加入的人都看得到，只有房主改得动（`canChange`）。 */
export function useSubRoomPolicy(
  gateway: SubRoomGateway | undefined,
  parentId: string,
  enabled = true,
) {
  return useQuery({
    enabled: enabled && gateway !== undefined,
    queryKey: subRoomPolicyQueryKey(parentId),
    queryFn: async () =>
      gateway === undefined ? err(unavailable) : await gateway.policy(parentId),
    networkMode: 'always',
    retry: false,
    staleTime: 5_000,
  });
}
