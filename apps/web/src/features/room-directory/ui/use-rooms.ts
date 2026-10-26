import { useQueries, useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { useMemo } from 'react';

import { useAppServices } from '@/app/app-services';
import { PrivateRoomCoordinator } from '@/features/private-rooms/application/private-room-coordinator';
import {
  privateRoomListQueryKey,
  privateRoomOriginQueryOptions,
  privateRoomPeopleQueryOptions,
  usePrivateRoomList,
  useServerReadableRooms,
} from '@/features/private-rooms/data/private-room-queries';
import type {
  CreatePrivateRoomInput,
  PrivateRoom,
  PrivateRoomFailure,
} from '@/features/private-rooms/domain/private-room';
import {
  groupMyPrivateRooms,
  roomOwnerOf,
  type GroupedPrivateRooms,
  type RoomOwner,
} from '@/features/room-directory/domain/my-private-rooms';
import { useOptionalSession } from '@/features/session/ui/session-provider';
import type { Result } from '@/shared/result';

export type RoomListState = 'failed' | 'loading' | 'ready';

export type MyPrivateRooms = GroupedPrivateRooms & {
  readonly principalId: string | undefined;
  /** 邀请里哪些房间是网络 Agent 建的、服务器读得到（房间 ID）；接受之前就要说。 */
  readonly serverReadable: ReadonlySet<string>;
  readonly state: RoomListState;
  readonly retry: () => void;
};

/** 你的私人房间：受邀还没答复的、已经加入的，和归档了的（只算你加入过的）。 */
export function useMyPrivateRooms(): MyPrivateRooms {
  const { privateRooms } = useAppServices();
  const principalId = useOptionalSession()?.snapshot.context.principal?.principalId;
  const query = usePrivateRoomList(privateRooms, principalId);
  const grouped = groupMyPrivateRooms(query.data?.ok === true ? query.data.value : [], principalId);
  const serverReadable = useServerReadableRooms(
    privateRooms,
    grouped.invitations.map((room) => room.catalogId),
  );
  return {
    ...grouped,
    principalId,
    serverReadable,
    state: query.isPending ? 'loading' : query.isError || !query.data.ok ? 'failed' : 'ready',
    retry: () => void query.refetch(),
  };
}

/**
 * 这些房间的房主（房间 ID → 房主），只给“房间”页上别人的房间用：每间问一次是谁建的、问一次成员的昵称，
 * 两个一起问。没问到、没问成的不在里面，卡片先不写房主；名字随房间列表一起刷新。
 */
export function useRoomOwners(rooms: readonly PrivateRoom[]): ReadonlyMap<string, RoomOwner> {
  const { privateRooms } = useAppServices();
  const origins = useQueries({
    queries: rooms.map((room) => privateRoomOriginQueryOptions(privateRooms, room.catalogId)),
    combine: (results) =>
      results.map((result) => (result.data?.ok === true ? result.data.value : undefined)),
  });
  const people = useQueries({
    queries: rooms.map((room) => privateRoomPeopleQueryOptions(privateRooms, room.catalogId)),
    combine: (results) =>
      results.map((result) => (result.data?.ok === true ? result.data.value : undefined)),
  });
  const owners = new Map<string, RoomOwner>();
  rooms.forEach((room, index) => {
    const owner = roomOwnerOf(room, origins[index], people[index]);
    if (owner !== null) owners.set(room.catalogId, owner);
  });
  return owners;
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
