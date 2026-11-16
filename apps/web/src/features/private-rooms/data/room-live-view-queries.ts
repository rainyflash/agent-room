import { queryOptions, useQuery } from '@tanstack/react-query';

import { privateRoomListQueryKey } from '@/features/private-rooms/data/private-room-queries';
import type { RoomLiveViewGateway } from '@/features/private-rooms/domain/room-live-view';
import { err } from '@/shared/result';

/** 测试和夹具里没有这项服务时不去问；查询本来也不会跑。 */
const unavailable = { code: 'room_live.unavailable', retryable: false } as const;

/**
 * 这间房的实况开没开、此刻放没放。挂在房间列表的键下面：公开设置存好（改回私密、换加入方式时服务器会自己
 * 关掉实况）、归档、退出以后房间列表重读，这份跟着重问。设置里的开关和顶栏那一行读的是同一份。
 */
export const roomLiveViewQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'live-view', catalogId] as const;

export function useRoomLiveView(
  gateway: RoomLiveViewGateway | undefined,
  catalogId: string,
  enabled = true,
) {
  return useQuery(
    queryOptions({
      enabled: enabled && gateway !== undefined,
      queryKey: roomLiveViewQueryKey(catalogId),
      queryFn: async () =>
        gateway === undefined ? err(unavailable) : await gateway.liveView(catalogId),
      networkMode: 'always',
      retry: false,
      staleTime: 30_000,
    }),
  );
}
