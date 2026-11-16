import { describe, expect, it } from 'vitest';

import type { RoomUnread } from '@/features/inbox/domain/inbox';

import { familyUnread } from './sub-room-unread';

const PARENT = 'parent';
const SPECS = 'specs';
const RETRO = 'retro';
const ELSEWHERE = 'elsewhere';

const unread = (
  catalogId: string,
  count: number | null,
  mentioned = false,
  more = false,
): RoomUnread => ({ catalogId, count, mentioned, more });

const rooms = new Map<string, RoomUnread>([
  ['!parent', unread(PARENT, 3)],
  ['!specs', unread(SPECS, 2)],
  ['!retro', unread(RETRO, 5, true, true)],
  ['!elsewhere', unread(ELSEWHERE, 9, true)],
]);
const family = new Set([SPECS, RETRO]);

describe('“房间”页卡片上一整间房的未读', () => {
  it('把你在的子房间的没读的加进来，别的房间的不算', () => {
    expect(familyUnread(rooms.get('!parent'), family, rooms, new Set())).toEqual({
      count: 10,
      mentioned: true,
      more: true,
    });
  });

  it('免打扰的子房间不数没读的，提到你的照样算', () => {
    expect(familyUnread(rooms.get('!parent'), family, rooms, new Set(['!retro']))).toEqual({
      count: 5,
      mentioned: true,
      more: false,
    });
  });

  it('没有读到的位置的不数；这间房自己还没读出来时照子房间的算', () => {
    const fresh = new Map(rooms).set('!specs', unread(SPECS, null));
    expect(familyUnread(undefined, new Set([SPECS]), fresh, new Set())).toEqual({
      count: null,
      mentioned: false,
      more: false,
    });
    expect(familyUnread(undefined, family, rooms, new Set(['!retro']))).toEqual({
      count: 2,
      mentioned: true,
      more: false,
    });
  });

  it('收件箱里一间都没有时什么都不写', () => {
    expect(familyUnread(undefined, new Set(['gone']), rooms, new Set())).toBeUndefined();
  });
});
