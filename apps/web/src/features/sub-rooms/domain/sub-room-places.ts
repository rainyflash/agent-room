import type {
  SubRoom,
  SubRoomCategory,
  SubRoomList,
} from '@/features/private-rooms/domain/sub-rooms';

/** 左栏里的一组：`category` 为空的是“未分类”，排在最上面、不带标题。 */
export type SubRoomPlaceGroup = {
  readonly category: SubRoomCategory | null;
  readonly rooms: readonly SubRoom[];
};

/**
 * 左栏里的子房间（`specs/redesign/sub-rooms.md`“网页”）：只放你在的，加上眼前这一间（你在看它就算），
 * 按分类分组；一个分类里一间都没有就不出现。分类按先后，每组里照名单的顺序（建的先后）。
 */
export function subRoomPlaceGroups(
  list: SubRoomList,
  currentCatalogId: string,
): readonly SubRoomPlaceGroup[] {
  const shown = list.subRooms.filter((room) => room.joined || room.catalogId === currentCatalogId);
  const known = new Set(list.categories.map((category) => category.categoryId));
  const uncategorized = shown.filter(
    (room) => room.categoryId === undefined || !known.has(room.categoryId),
  );
  const groups: SubRoomPlaceGroup[] =
    uncategorized.length === 0 ? [] : [{ category: null, rooms: uncategorized }];
  for (const category of [...list.categories].sort((a, b) => a.position - b.position)) {
    const rooms = shown.filter((room) => room.categoryId === category.categoryId);
    if (rooms.length > 0) groups.push({ category, rooms });
  }
  return groups;
}

/** 眼前这间房是不是子房间：名单里的父房间不是它自己。 */
export function isSubRoomOf(list: SubRoomList, catalogId: string): boolean {
  return list.parent.catalogId !== catalogId;
}
