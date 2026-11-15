import { useQueryClient } from '@tanstack/react-query';
import { useCallback, useMemo, useState } from 'react';

import { useAppServices } from '@/app/app-services';
import { privateRoomListQueryKey } from '@/features/private-rooms/data/private-room-queries';
import { subRoomsQueryKey } from '@/features/private-rooms/data/sub-room-queries';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type { SubRoom } from '@/features/private-rooms/domain/sub-rooms';
import { useRoomCommands } from '@/features/room-directory/ui/use-rooms';
import { err, type Result } from '@/shared/result';

export type SubRoomActionKind = 'join' | 'request' | 'withdraw' | 'restore';

/** “浏览子房间”上正在做的、刚才没做成的：按子房间记，每一行看它。 */
export type SubRoomActions = {
  readonly busy: { readonly catalogId: string; readonly kind: SubRoomActionKind } | null;
  readonly failure: { readonly catalogId: string; readonly code: string } | null;
  readonly join: (room: SubRoom) => void;
  readonly request: (room: SubRoom, message: string) => void;
  readonly withdraw: (room: SubRoom) => void;
  readonly restore: (room: SubRoom) => void;
};

const unavailable: PrivateRoomFailure = { code: 'sub_room.unavailable', retryable: false };

/**
 * 一间房下面的子房间上能做的事。加入（没有门的，或者申请时门刚拆掉）以后，网页照“进私人房间”那一套
 * 在 Matrix 上加入、跳进去；申请、撤回、恢复以后重读子房间的名单。
 */
export function useSubRoomActions(parentId: string): SubRoomActions {
  const { privateRooms, subRooms } = useAppServices();
  const commands = useRoomCommands();
  const queries = useQueryClient();
  const [busy, setBusy] = useState<SubRoomActions['busy']>(null);
  const [failure, setFailure] = useState<SubRoomActions['failure']>(null);

  const run = useCallback(
    (
      room: SubRoom,
      kind: SubRoomActionKind,
      action: () => Promise<Result<unknown, PrivateRoomFailure>>,
    ) => {
      setBusy({ catalogId: room.catalogId, kind });
      setFailure(null);
      void (async () => {
        const result = await action();
        setBusy(null);
        if (!result.ok) setFailure({ catalogId: room.catalogId, code: result.error.code });
        // 恢复以后这一间自己的页面也要重读（“已经归档”那一行收起来），所以连房间一起刷新。
        await queries.invalidateQueries({
          queryKey: kind === 'restore' ? privateRoomListQueryKey : subRoomsQueryKey,
        });
      })();
    },
    [queries],
  );

  return useMemo(() => {
    const ref = (room: SubRoom) => ({ parentId, subRoomId: room.catalogId });
    return {
      busy,
      failure,
      join: (room) => {
        run(room, 'join', async () => {
          if (subRooms === undefined) return err(unavailable);
          const joined = await subRooms.join(ref(room));
          // 服务器已经记成已加入、在 Matrix 上替你发了邀请：接着照进私人房间那样加入、跳进去。
          return joined.ok ? await commands.enter(joined.value, false) : joined;
        });
      },
      request: (room, message) => {
        run(room, 'request', async () => {
          if (subRooms === undefined) return err(unavailable);
          const requested = await subRooms.requestToJoin(ref(room), message.trim());
          if (!requested.ok || requested.value.membership === 'pending') return requested;
          // 门刚拆掉、你管父房间：不用等，已经记成已加入。
          const inspected = await privateRooms.inspect(room.catalogId);
          return inspected.ok ? await commands.enter(inspected.value, false) : inspected;
        });
      },
      withdraw: (room) => {
        run(room, 'withdraw', async () =>
          subRooms === undefined ? err(unavailable) : await subRooms.withdrawJoinRequest(ref(room)),
        );
      },
      restore: (room) => {
        run(room, 'restore', async () =>
          subRooms === undefined ? err(unavailable) : await subRooms.restore(ref(room)),
        );
      },
    } satisfies SubRoomActions;
  }, [busy, commands, failure, parentId, privateRooms, run, subRooms]);
}
