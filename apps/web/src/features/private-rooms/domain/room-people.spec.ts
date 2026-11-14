import { describe, expect, it } from 'vitest';

import type { PrivateRoom, PrivateRoomMember } from './private-room';
import { roomPeople } from './room-people';

const id = (suffix: number) => `0198b601-77a1-7bb8-83eb-a8fe68c9${String(suffix).padStart(4, '0')}`;
const OWNER = id(1);
const ADMIN = id(2);
const MEMBER = id(3);
const INVITED = id(4);
const BANNED = id(5);
const UNNAMED = id(6);

const member = (
  principalId: string,
  status: PrivateRoomMember['status'],
  capabilities: PrivateRoomMember['permissions']['capabilities'] = ['view', 'speak'],
): PrivateRoomMember => ({ permissions: { capabilities }, principalId, status });

const ROOM = {
  catalogId: id(100),
  description: '',
  matrixRoomId: '!private:matrix.test',
  members: [
    member(MEMBER, 'joined'),
    member(INVITED, 'invited'),
    member(ADMIN, 'joined', ['view', 'speak', 'manage']),
    member(BANNED, 'banned'),
    member(OWNER, 'joined', ['view', 'speak', 'invite', 'manage', 'automate']),
    member(UNNAMED, 'joined'),
  ],
  name: 'Sync probe',
  ownerPrincipalId: OWNER,
  retentionDays: 30,
  roomInstanceId: id(101),
  status: 'active',
  version: 0,
} satisfies PrivateRoom;

const NAMES = [
  { displayName: 'Zed', principalId: MEMBER },
  { displayName: 'Ivy', principalId: INVITED },
  { displayName: 'Grace', principalId: ADMIN },
  { displayName: 'Mallory', principalId: BANNED },
  { displayName: 'Ada', principalId: OWNER },
  { displayName: '  ', principalId: UNNAMED },
];

const SPOKEN = [
  { matrixUserId: '@ada:matrix.test', displayName: 'Ada', isSelf: true },
  { matrixUserId: '@guest:matrix.test', displayName: 'Guest', isSelf: false },
];

describe('roomPeople', () => {
  it('有成员名单的房间：已加入的成员，房主在前、管理员其次，别的按名字', () => {
    const { listed, people } = roomPeople(ROOM, NAMES, MEMBER, SPOKEN);
    expect(listed).toBe(true);
    expect(people.map((person) => [person.displayName, person.role, person.self])).toEqual([
      ['Ada', 'owner', false],
      ['Grace', 'admin', false],
      ['Zed', null, true],
    ]);
    expect(people.map((person) => person.principalId)).toEqual([OWNER, ADMIN, MEMBER]);
  });

  it('受邀还没来的、被封禁的、没有昵称的（替网络 Agent 记账的主体）不列', () => {
    const names = roomPeople(ROOM, NAMES, OWNER, SPOKEN).people.map((person) => person.displayName);
    expect(names).not.toContain('Ivy');
    expect(names).not.toContain('Mallory');
    expect(names).toHaveLength(3);
  });

  it('公共大厅没有成员名单：列说过话的人和你自己，不写身份', () => {
    const { listed, people } = roomPeople(undefined, null, OWNER, SPOKEN);
    expect(listed).toBe(false);
    expect(people).toEqual([
      { key: '@ada:matrix.test', displayName: 'Ada', self: true, role: null, principalId: null },
      {
        key: '@guest:matrix.test',
        displayName: 'Guest',
        self: false,
        role: null,
        principalId: null,
      },
    ]);
  });

  it('名单还没读到时先照说过话的人列', () => {
    expect(roomPeople(ROOM, null, OWNER, SPOKEN).listed).toBe(false);
  });
});
