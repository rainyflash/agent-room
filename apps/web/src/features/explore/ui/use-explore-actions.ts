import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useMemo, useState } from 'react';

import { useAppServices } from '@/app/app-services';
import { exploreQueryKey } from '@/features/explore/data/explore-queries';
import type { ExploreRoom } from '@/features/explore/domain/explore';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type { PublicRoomMembership } from '@/features/private-rooms/domain/public-room-join';
import { useRoomCommands } from '@/features/room-directory/ui/use-rooms';
import { err, ok, type Result } from '@/shared/result';

export type ExploreActionKind = 'join' | 'request' | 'withdraw';

/** 发现页上正在做的、刚才没做成的：按房间记，卡片和详情都看它。 */
export type ExploreActions = {
  readonly busy: { readonly catalogId: string; readonly kind: ExploreActionKind } | null;
  readonly failure: { readonly catalogId: string; readonly code: string } | null;
  readonly onJoin: (room: ExploreRoom) => void;
  readonly onRequest: (room: ExploreRoom, message: string) => void;
  readonly onWithdraw: (room: ExploreRoom) => void;
};

const unavailable: PrivateRoomFailure = { code: 'room_join.unavailable', retryable: false };

/**
 * 从发现页进公开房间。直接进的服务器以房主的名义邀请你，网页接着照接受邀请那一套在 Matrix 上加入、
 * 进房间；收到过邀请的直接接受。申请、撤回以后重读目录，卡片和详情上的处境跟着变。
 */
export function useExploreActions(): ExploreActions {
  const { privateRooms, publicRoomEntry } = useAppServices();
  const commands = useRoomCommands();
  const queries = useQueryClient();
  const [busy, setBusy] = useState<ExploreActions['busy']>(null);
  const [failure, setFailure] = useState<ExploreActions['failure']>(null);

  const enter = useCallback(
    async (membership: PublicRoomMembership): Promise<Result<unknown, PrivateRoomFailure>> => {
      if (membership.membership === 'pending') return ok(undefined);
      const room = await privateRooms.inspect(membership.catalogId);
      if (!room.ok) return room;
      return await commands.enter(room.value, membership.membership === 'invited');
    },
    [commands, privateRooms],
  );

  const run = useCallback(
    (
      room: ExploreRoom,
      kind: ExploreActionKind,
      action: () => Promise<Result<unknown, PrivateRoomFailure>>,
    ) => {
      setBusy({ catalogId: room.catalogId, kind });
      setFailure(null);
      void (async () => {
        const result = await action();
        setBusy(null);
        if (!result.ok) setFailure({ catalogId: room.catalogId, code: result.error.code });
        await queries.invalidateQueries({ queryKey: exploreQueryKey });
      })();
    },
    [queries],
  );

  return useMemo(
    () => ({
      busy,
      failure,
      onJoin: (room) => {
        run(room, 'join', async () => {
          // 收到过邀请的不用再问服务器，直接接受；只能邀请的房间问了也是拒绝。
          if (room.viewer === 'invited') {
            return await enter({ catalogId: room.catalogId, membership: 'invited' });
          }
          if (publicRoomEntry === undefined) return err(unavailable);
          const joined = await publicRoomEntry.join(room.catalogId);
          return joined.ok ? await enter(joined.value) : joined;
        });
      },
      onRequest: (room, message) => {
        run(room, 'request', async () => {
          if (publicRoomEntry === undefined) return err(unavailable);
          const requested = await publicRoomEntry.requestToJoin(room.catalogId, message.trim());
          return requested.ok ? await enter(requested.value) : requested;
        });
      },
      onWithdraw: (room) => {
        run(room, 'withdraw', async () =>
          publicRoomEntry === undefined
            ? err(unavailable)
            : await publicRoomEntry.withdrawJoinRequest(room.catalogId),
        );
      },
    }),
    [busy, enter, failure, publicRoomEntry, run],
  );
}
