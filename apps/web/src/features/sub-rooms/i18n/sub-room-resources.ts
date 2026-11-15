/** 子房间（`specs/redesign/sub-rooms.md`）。 */
const en = {
  'subRooms.places.label': 'Sub-rooms',
  'subRooms.places.gated': 'Has a door',
  'subRooms.places.unread': '{{count}} unread',
  'subRooms.places.unread_one': '{{count}} unread',
  'subRooms.places.unread_other': '{{count}} unread',
  'subRooms.places.mentioned': 'Mentions you',
} as const;

export const subRoomResources = {
  en,
  'zh-CN': {
    'subRooms.places.label': '子房间',
    'subRooms.places.gated': '有门',
    'subRooms.places.unread': '{{count}} 条没读',
    'subRooms.places.unread_one': '{{count}} 条没读',
    'subRooms.places.unread_other': '{{count}} 条没读',
    'subRooms.places.mentioned': '有人提到你',
  } satisfies Record<keyof typeof en, string>,
} as const;
