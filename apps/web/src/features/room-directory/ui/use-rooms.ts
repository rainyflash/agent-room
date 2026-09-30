import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useMemo } from 'react';

import { useAppServices } from '@/app/app-services';
import { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import {
  privateRoomListQueryKey,
  usePrivateRoomList,
} from '@/features/private-rooms/data/private-room-queries';
import type {
  CreatePrivateRoomInput,
  PrivateRoom,
  PrivateRoomFailure,
} from '@/features/private-rooms/domain/private-room';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import type { Result } from '@/shared/result';

export type RoomListState = 'failed' | 'loading' | 'ready';

export type MyPrivateRooms = {
  readonly principalId: string | undefined;
  /** 有人邀请了你、你还没答复的房间。 */
  readonly invitations: readonly PrivateRoom[];
  readonly joined: readonly PrivateRoom[];
  readonly state: RoomListState;
  readonly retry: () => void;
};

export function isInvitedTo(room: PrivateRoom, principalId: string | undefined): boolean {
  return room.members.find((member) => member.principalId === principalId)?.status === 'invited';
}

/** 你的私人房间：受邀还没答复的，和已经加入的；归档了的不算。 */
export function useMyPrivateRooms(): MyPrivateRooms {
  const { privateRooms } = useAppServices();
  const principalId = useOptionalSession()?.snapshot.context.principal?.principalId;
  const query = usePrivateRoomList(privateRooms, principalId);
  const active =
    query.data?.ok === true ? query.data.value.filter((room) => room.status === 'active') : [];
  const membership = (room: PrivateRoom) =>
    room.members.find((member) => member.principalId === principalId)?.status;
  return {
    principalId,
    invitations: active.filter((room) => membership(room) === 'invited'),
    joined: active.filter((room) => membership(room) === 'joined'),
    state: query.isPending ? 'loading' : query.isError || !query.data.ok ? 'failed' : 'ready',
    retry: () => void query.refetch(),
  };
}

export type RoomResult = Result<PrivateRoom, PrivateRoomFailure>;

export type RoomCommands = {
  /** 进一个私人房间；受邀还没答复的，先接受邀请。成功后直接跳过去。 */
  readonly enter: (room: PrivateRoom, invited: boolean) => Promise<RoomResult>;
  readonly decline: (room: PrivateRoom) => Promise<RoomResult>;
  /** 同一个请求 ID 重试不会再建一个房间。成功后直接跳进新房间。 */
  readonly create: (requestId: string, input: CreatePrivateRoomInput) => Promise<RoomResult>;
};

export function useRoomCommands(): RoomCommands {
  const { privateRoomMatrix, privateRooms } = useAppServices();
  const navigate = useNavigate();
  const queries = useQueryClient();
  const coordinator = useMemo(
    () => new PrivateRoomCoordinator(privateRooms, privateRoomMatrix),
    [privateRoomMatrix, privateRooms],
  );
  return useMemo(() => {
    const settle = async (result: RoomResult): Promise<RoomResult> => {
      await queries.invalidateQueries({ queryKey: privateRoomListQueryKey });
      return result;
    };
    const go = async (result: RoomResult): Promise<RoomResult> => {
      if (result.ok) {
        await navigate({
          to: '/lobby/$catalogId/instance/$roomId',
          params: { catalogId: result.value.catalogId, roomId: result.value.matrixRoomId },
          search: {},
        });
      }
      return result;
    };
    return {
      enter: async (room, invited) =>
        await go(await settle(await (invited ? coordinator.accept(room) : coordinator.open(room)))),
      decline: async (room) => await settle(await coordinator.decline(room)),
      create: async (requestId, input) =>
        await go(await settle(await coordinator.createAndJoin(requestId, input))),
    };
  }, [coordinator, navigate, queries]);
}
