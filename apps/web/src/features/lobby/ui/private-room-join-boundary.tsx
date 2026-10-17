import { useQuery } from '@tanstack/react-query';
import { useEffect, useState } from 'react';

import { useAppServices } from '@/app/app-services';
import type { LobbyRoomState } from '@/features/lobby/application/lobby-room-store';
import { LobbyStateBoundary } from '@/features/lobby/ui/lobby-state-boundary';
import { usePrivateRoomList } from '@/features/private-rooms/data/private-room-queries';
import { joinedPrivateRoom } from '@/features/private-rooms/domain/private-room';

/** 加入请求回来以后，最多等这么久让同步把“已加入”带回来；过了还没到，照常说打不开、能重试。 */
export const PRIVATE_ROOM_JOIN_SETTLE_MS = 15_000;

export type PrivateRoomJoinBoundaryProps = {
  readonly catalogId: string;
  readonly onRetry: () => void;
  readonly principalId: string | null;
  readonly roomId: string;
  readonly state: Extract<LobbyRoomState, { readonly kind: 'failed' }>;
};

/**
 * Matrix 说你还不在这个房间里，服务器却记着你已经在了（房主、接受过邀请的人）：替你加入，这段时间照常显示
 * “正在打开”，同步带回来以后房间自己打开。本机 Agent 替你建的房间就是这样：建房时你在 Matrix 上只是受邀，
 * 从房间列表、提醒或者地址直接进来都不经过“加入”。服务器没记你在里面（还没答复的邀请、别人的房间），
 * 或者加入没成、迟迟没同步回来，才说打不开；这时“重试”会再加入一次。
 */
export function PrivateRoomJoinBoundary({
  catalogId,
  onRetry,
  principalId,
  roomId,
  state,
}: PrivateRoomJoinBoundaryProps) {
  const { privateRoomMatrix, privateRooms } = useAppServices();
  const list = usePrivateRoomList(privateRooms, principalId ?? undefined, principalId !== null);
  const room =
    principalId !== null && list.data?.ok === true
      ? joinedPrivateRoom(list.data.value, catalogId, roomId, principalId)
      : null;
  const join = useQuery({
    enabled: room !== null,
    // 房间打开以后这一页就卸了；下次再遇到没加入时要重新加入，不沿用这次的结果。
    gcTime: 0,
    networkMode: 'always',
    queryFn: async () => await privateRoomMatrix.join(roomId),
    queryKey: ['matrix', 'private-room-join', roomId] as const,
    retry: false,
    staleTime: Infinity,
  });
  const settling = useSettling(join.data?.ok === true ? join.dataUpdatedAt : null);
  const opening =
    (principalId !== null && list.isPending) ||
    (room !== null && (join.isPending || join.isFetching || settling));
  return (
    <LobbyStateBoundary
      onRetry={() => {
        if (room !== null) void join.refetch();
        onRetry();
      }}
      state={opening ? { kind: 'loading' } : state}
    />
  );
}

/** 加入请求在 `since` 这一刻回来以后的一小段时间里是 true；再加入一次重新算。 */
function useSettling(since: number | null): boolean {
  const [expired, setExpired] = useState<number | null>(null);
  useEffect(() => {
    if (since === null) return;
    const timer = setTimeout(() => {
      setExpired(since);
    }, PRIVATE_ROOM_JOIN_SETTLE_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [since]);
  return since !== null && expired !== since;
}
