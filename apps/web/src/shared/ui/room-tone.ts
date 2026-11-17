/**
 * 房间小图标的颜色按房间号挑：同一间房在“房间”页、“换个房间”和发现页都是同一种。
 * 用的地方给元素加 `ar-tile` 和 `data-tone`，颜色在 ui-system 的样式里。
 */
export const roomTones = ['teal', 'coral', 'sunny', 'sky'] as const;
export type RoomTone = (typeof roomTones)[number];

export function roomTone(catalogId: string): RoomTone {
  let hash = 0;
  for (const character of catalogId) {
    hash = (hash * 31 + (character.codePointAt(0) ?? 0)) >>> 0;
  }
  return roomTones[hash % roomTones.length] ?? 'teal';
}
