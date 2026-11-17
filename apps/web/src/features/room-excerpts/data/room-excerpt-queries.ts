import { queryOptions, useQuery } from '@tanstack/react-query';

import { privateRoomListQueryKey } from '@/features/private-rooms/data/private-room-queries';
import type { RoomExcerptGateway } from '@/features/room-excerpts/domain/room-excerpts';
import { err } from '@/shared/result';

/** 测试和夹具里没有这项服务时不去问；查询本来也不会跑。 */
const unavailable = { code: 'room_excerpt.unavailable', retryable: false } as const;

/**
 * 这间房分享过的几段（自己的；房主和管理员是全部）。挂在房间列表的键下面：公开设置存好、归档、退出以后
 * 房间列表重读，这份跟着重问（改了加入方式、改回私密时服务器会关掉实况、一起删掉分享过的段）。
 */
export const roomExcerptsQueryKey = (catalogId: string) =>
  [...privateRoomListQueryKey, 'excerpts', catalogId] as const;

export function useRoomExcerpts(
  gateway: RoomExcerptGateway | undefined,
  catalogId: string,
  enabled = true,
) {
  return useQuery(
    queryOptions({
      enabled: enabled && gateway !== undefined,
      queryKey: roomExcerptsQueryKey(catalogId),
      queryFn: async () =>
        gateway === undefined ? err(unavailable) : await gateway.list(catalogId),
      networkMode: 'always',
      retry: false,
      staleTime: 30_000,
    }),
  );
}
