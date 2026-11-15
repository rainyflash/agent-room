import { describe, expect, it } from 'vitest';

import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';

import {
  searchSubRooms,
  subRoomBrowseGroups,
  subRoomRowAction,
  subRoomsOnTab,
  unseenSubRooms,
} from './sub-room-browse';

const WORK = '0198b601-77a1-7bb8-83eb-a8fe68c97e60';
const OPS = '0198b601-77a1-7bb8-83eb-a8fe68c97e61';
const GONE = '0198b601-77a1-7bb8-83eb-a8fe68c97e69';

function subRoom(id: string, name: string, extra: Partial<SubRoom> = {}): SubRoom {
  return {
    catalogId: `0198b601-77a1-7bb8-83eb-a8fe68c97e${id}`,
    name,
    description: '',
    access: 'open',
    hidden: false,
    createdByPrincipalId: '0198b601-77a1-7bb8-83eb-a8fe68c97e42',
    createdAtUnixMs: 1,
    managerPrincipalIds: [],
    joined: false,
    banned: false,
    ...extra,
  };
}

const categories = [
  { categoryId: OPS, name: 'Ops', position: 1 },
  { categoryId: WORK, name: 'Work', position: 0 },
];

describe('“浏览子房间”每一行的动作', () => {
  it('你在的是已加入；被封禁的什么也不给；在等的能撤回；其余看门', () => {
    expect(subRoomRowAction(subRoom('51', 'a', { joined: true, access: 'request' }))).toBe(
      'joined',
    );
    expect(subRoomRowAction(subRoom('52', 'b', { banned: true }))).toBe('banned');
    expect(subRoomRowAction(subRoom('53', 'c', { access: 'request', requestedAtUnixMs: 5 }))).toBe(
      'pending',
    );
    expect(subRoomRowAction(subRoom('54', 'd'))).toBe('join');
    expect(subRoomRowAction(subRoom('55', 'e', { access: 'request' }))).toBe('request');
    expect(subRoomRowAction(subRoom('56', 'f', { access: 'invite_only' }))).toBe('invite_only');
  });
});

describe('搜和标签', () => {
  const rooms = [
    subRoom('51', 'Release Alpha 95', { description: 'What ships', joined: true }),
    subRoom('52', 'Backups', { description: 'Restore drills' }),
  ];

  it('名字和用途都搜，不分大小写，几个词都要有', () => {
    expect(searchSubRooms(rooms, 'alpha').map((room) => room.name)).toEqual(['Release Alpha 95']);
    expect(searchSubRooms(rooms, 'RESTORE').map((room) => room.name)).toEqual(['Backups']);
    expect(searchSubRooms(rooms, 'release ships')).toHaveLength(1);
    expect(searchSubRooms(rooms, 'release drills')).toHaveLength(0);
    expect(searchSubRooms(rooms, '   ')).toBe(rooms);
  });

  it('“我在的”只留你在的', () => {
    expect(subRoomsOnTab(rooms, 'mine').map((room) => room.name)).toEqual(['Release Alpha 95']);
    expect(subRoomsOnTab(rooms, 'all')).toBe(rooms);
  });
});

describe('按分类分组', () => {
  it('分类按先后，空的不出现，“未分类”和认不得的分类排在最后', () => {
    const groups = subRoomBrowseGroups(categories, [
      subRoom('51', 'loose'),
      subRoom('52', 'ops', { categoryId: OPS }),
      subRoom('53', 'orphan', { categoryId: GONE }),
    ]);
    expect(groups.map((group) => group.category?.name ?? null)).toEqual(['Ops', null]);
    expect(groups[1]?.rooms.map((room) => room.name)).toEqual(['loose', 'orphan']);
  });
});

describe('没见过的新子房间', () => {
  const list: SubRoomList = {
    parent: { catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e40', name: 'Agent Room' },
    canCreate: true,
    categories,
    subRooms: [
      subRoom('51', 'seen'),
      subRoom('52', 'new'),
      subRoom('53', 'mine', { joined: true }),
    ],
  };

  it('这台设备上还没记过时不算；记过以后没见过、你又没进的才算', () => {
    expect(unseenSubRooms(list, null)).toBe(0);
    expect(unseenSubRooms(list, new Set([list.subRooms[0]?.catalogId ?? '']))).toBe(1);
  });
});
