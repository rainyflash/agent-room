import type {
  SubRoom,
  SubRoomCategory,
  SubRoomList,
} from '@/features/private-rooms/domain/sub-rooms';

/** “浏览子房间”一页的三个标签（`specs/redesign/sub-rooms.md`“网页”）。 */
export const subRoomBrowseTabs = ['all', 'mine', 'archived'] as const;
export type SubRoomBrowseTab = (typeof subRoomBrowseTabs)[number];

/** 一组：`category` 为空的是“未分类”，在这一页排在最后、带标题（左栏里排在最前、不带）。 */
export type SubRoomBrowseGroup = {
  readonly category: SubRoomCategory | null;
  readonly rooms: readonly SubRoom[];
};

/**
 * 每一行的那一个动作：`joined` 已加入（点名字进去），`join` 一点就进，`request` 申请加入，
 * `pending` 在等批准（能撤回），`invite_only` 只能请进来，`banned` 被这一间封禁了（什么也不给）。
 */
export type SubRoomRowAction = 'joined' | 'join' | 'request' | 'pending' | 'invite_only' | 'banned';

export function subRoomRowAction(room: SubRoom): SubRoomRowAction {
  if (room.joined) return 'joined';
  if (room.banned) return 'banned';
  if (room.requestedAtUnixMs !== undefined) return 'pending';
  if (room.access === 'open') return 'join';
  return room.access === 'request' ? 'request' : 'invite_only';
}

/** 搜名字和用途：不分大小写，空格分开的几个词都要有。 */
export function searchSubRooms(rooms: readonly SubRoom[], query: string): readonly SubRoom[] {
  const words = query.toLocaleLowerCase().split(/\s+/u).filter(Boolean);
  if (words.length === 0) return rooms;
  return rooms.filter((room) => {
    const text = `${room.name}\n${room.description}`.toLocaleLowerCase();
    return words.every((word) => text.includes(word));
  });
}

/** 标签里的子房间：“我在的”只留你在的；“全部”和“已归档”照名单。 */
export function subRoomsOnTab(
  rooms: readonly SubRoom[],
  tab: Exclude<SubRoomBrowseTab, 'archived'>,
): readonly SubRoom[] {
  return tab === 'mine' ? rooms.filter((room) => room.joined) : rooms;
}

/**
 * 按分类分组：分类按先后，每组里照名单的顺序；没有一间的分类不出现。不认得的分类（刚被删）算“未分类”，
 * “未分类”排在最后。一个分类都没有时只有一组、不带标题。
 */
export function subRoomBrowseGroups(
  categories: readonly SubRoomCategory[],
  rooms: readonly SubRoom[],
): readonly SubRoomBrowseGroup[] {
  const known = new Set(categories.map((category) => category.categoryId));
  const groups: SubRoomBrowseGroup[] = [];
  for (const category of [...categories].sort((a, b) => a.position - b.position)) {
    const inside = rooms.filter((room) => room.categoryId === category.categoryId);
    if (inside.length > 0) groups.push({ category, rooms: inside });
  }
  const loose = rooms.filter(
    (room) => room.categoryId === undefined || !known.has(room.categoryId),
  );
  if (loose.length > 0) groups.push({ category: null, rooms: loose });
  return groups;
}

/** 归档的那一间原来在哪个分类：分类还在就写名字，不在了算“未分类”。 */
export function subRoomCategoryName(
  categories: readonly SubRoomCategory[],
  room: SubRoom,
): string | null {
  return categories.find((category) => category.categoryId === room.categoryId)?.name ?? null;
}

/**
 * 左栏“浏览子房间”那一行的点：有你没进、这台设备上又没见过的子房间。`seen` 为空（这台设备上还没记过）
 * 时不算新的：头一回打开时名单上的都当见过了，不然人一来就看到一个点。
 */
export function unseenSubRooms(list: SubRoomList, seen: ReadonlySet<string> | null): number {
  if (seen === null) return 0;
  return list.subRooms.filter((room) => !room.joined && !seen.has(room.catalogId)).length;
}
