import { useMemo } from 'react';

import { useAppServices } from '@/app/app-services';
import type { RoomUnread } from '@/features/inbox/domain/inbox';
import { useInbox } from '@/features/inbox/ui/inbox-provider';
import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { useSubRooms } from '@/features/private-rooms/data/sub-room-queries';
import type { PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { familyUnread, type FamilyUnread } from '@/features/sub-rooms/domain/sub-room-unread';

const noRooms: ReadonlyMap<string, RoomUnread> = new Map<string, RoomUnread>();
const noneMuted: ReadonlySet<string> = new Set();

/**
 * “房间”页上一间房卡片的未读和 @，把你在的子房间的也算进去（`familyUnread`）。子房间的名单和房间页左栏
 * 问的是同一份；归档的房间不去问。收件箱还没读出来时是 undefined。
 */
export function useFamilyUnread(room: PrivateRoom, enabled: boolean): FamilyUnread | undefined {
  const { subRooms } = useAppServices();
  const inbox = useInbox();
  const muted = usePersonalWorkspace()?.snapshot.index.muted ?? noneMuted;
  const family = useSubRooms(subRooms, room.catalogId, { enabled });
  const list = family.data?.ok === true ? family.data.value : null;
  const subRoomIds = useMemo(
    () =>
      new Set(
        // 问的是这间房自己：它要是别人的子房间，问回来的是父房间那一家，不算。
        list?.parent.catalogId === room.catalogId
          ? list.subRooms.map((entry) => entry.catalogId)
          : [],
      ),
    [list, room.catalogId],
  );
  const rooms = inbox?.snapshot.rooms ?? noRooms;
  return familyUnread(rooms.get(room.matrixRoomId), subRoomIds, rooms, muted);
}
