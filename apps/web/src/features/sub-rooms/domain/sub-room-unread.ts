import type { RoomUnread } from '@/features/inbox/domain/inbox';

/** “房间”页一张卡片上的未读：这间房自己的，加上你在的子房间的。 */
export type FamilyUnread = {
  /** 加起来几条没读；这间房和子房间都还没有读到的位置（从没打开过）时是 null。 */
  readonly count: number | null;
  readonly mentioned: boolean;
  /** 有一间本机的消息没接到读到的位置：真实的只会更多。 */
  readonly more: boolean;
};

/**
 * “房间”页的卡片只有父房间一张（`specs/redesign/sub-rooms.md`“看得到什么”）：没读的和 @ 把你在的子房间的
 * 也算进去。免打扰的子房间不数没读的，提到你的照样算。
 *
 * `rooms` 是收件箱按 Matrix 房间号排的未读，只有你在的房间；`subRoomIds` 是这间房下面没归档的子房间，
 * 归档的不在里面、不算。收件箱里这间房和它的子房间一间都没有时是 undefined。
 */
export function familyUnread(
  parent: RoomUnread | undefined,
  subRoomIds: ReadonlySet<string>,
  rooms: ReadonlyMap<string, RoomUnread>,
  muted: ReadonlySet<string>,
): FamilyUnread | undefined {
  let found = parent !== undefined;
  let count = parent?.count ?? null;
  let mentioned = parent?.mentioned ?? false;
  let more = parent?.more ?? false;
  for (const [roomId, unread] of rooms) {
    if (!subRoomIds.has(unread.catalogId)) continue;
    found = true;
    mentioned ||= unread.mentioned;
    if (muted.has(roomId) || unread.count === null) continue;
    count = (count ?? 0) + unread.count;
    more ||= unread.more;
  }
  return found ? { count, mentioned, more } : undefined;
}
