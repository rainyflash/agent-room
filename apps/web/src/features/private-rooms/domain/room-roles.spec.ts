import { describe, expect, it } from 'vitest';

import {
  grantableBy,
  permissions,
  type PrivateRoom,
  type PrivateRoomCapability,
  type PrivateRoomMember,
} from './private-room';
import {
  assignableRoles,
  canManageMember,
  crossesAdmin,
  invitableBy,
  permissionsForRole,
  roomRole,
} from './room-roles';

const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const ADMIN = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';
const OTHER_ADMIN = '0198b601-77a1-7bb8-83eb-a8fe68c97e44';
const MEMBER = '0198b601-77a1-7bb8-83eb-a8fe68c97e45';
const VIEWER = '0198b601-77a1-7bb8-83eb-a8fe68c97e46';
const BANNED = '0198b601-77a1-7bb8-83eb-a8fe68c97e47';

function member(
  principalId: string,
  capabilities: PrivateRoomCapability[],
  status: PrivateRoomMember['status'] = 'joined',
): PrivateRoomMember {
  return { permissions: permissions(...capabilities), principalId, status };
}

const room: PrivateRoom = {
  catalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e40',
  description: '',
  matrixRoomId: '!roles:matrix.test',
  members: [
    member(OWNER, ['view', 'speak', 'invite', 'manage', 'automate']),
    member(ADMIN, ['view', 'speak', 'invite', 'manage']),
    member(OTHER_ADMIN, ['view', 'speak', 'manage', 'automate'], 'invited'),
    member(MEMBER, ['view', 'speak', 'automate']),
    member(VIEWER, ['view']),
    member(BANNED, [], 'banned'),
  ],
  name: 'Roles',
  ownerPrincipalId: OWNER,
  retentionDays: null,
  roomInstanceId: '0198b601-77a1-7bb8-83eb-a8fe68c97e41',
  status: 'active',
  version: 1,
};

function of(principalId: string): PrivateRoomMember {
  const found = room.members.find((entry) => entry.principalId === principalId);
  if (found === undefined) throw new Error('夹具里没有这个人。');
  return found;
}

describe('房间里的身份', () => {
  it('照权限算：房主、有“管理”的是管理员、能说的是成员、只能看的是只能看', () => {
    expect(roomRole(room, of(OWNER))).toBe('owner');
    expect(roomRole(room, of(ADMIN))).toBe('admin');
    expect(roomRole(room, of(OTHER_ADMIN))).toBe('admin');
    expect(roomRole(room, of(MEMBER))).toBe('member');
    expect(roomRole(room, of(VIEWER))).toBe('viewer');
  });

  it('房主能把别人换成三种里的任何一种，自己那一行换不了', () => {
    for (const target of [ADMIN, OTHER_ADMIN, MEMBER, VIEWER]) {
      expect(assignableRoles(room, OWNER, of(target))).toEqual(['admin', 'member', 'viewer']);
    }
    expect(assignableRoles(room, OWNER, of(OWNER))).toEqual([]);
    expect(assignableRoles(room, OWNER, of(BANNED))).toEqual([]);
  });

  it('管理员：动不了房主和别的管理员，成员在成员和只能看之间换，自己能降成成员', () => {
    expect(assignableRoles(room, ADMIN, of(OWNER))).toEqual([]);
    expect(assignableRoles(room, ADMIN, of(OTHER_ADMIN))).toEqual([]);
    expect(assignableRoles(room, ADMIN, of(MEMBER))).toEqual(['member', 'viewer']);
    expect(assignableRoles(room, ADMIN, of(VIEWER))).toEqual(['member', 'viewer']);
    expect(assignableRoles(room, ADMIN, of(ADMIN))).toEqual(['admin', 'member']);
  });

  it('成员谁的身份都换不了；受邀还没进来的管理员也换不了别人；归档了都换不了', () => {
    for (const target of [OWNER, ADMIN, MEMBER, VIEWER]) {
      expect(assignableRoles(room, MEMBER, of(target))).toEqual([]);
      expect(assignableRoles(room, OTHER_ADMIN, of(target))).toEqual([]);
      expect(assignableRoles({ ...room, status: 'archived' }, OWNER, of(target))).toEqual([]);
    }
  });

  it('自己没有“发言”的管理员没法把只能看的人改回成员', () => {
    const silent: PrivateRoom = {
      ...room,
      members: room.members.map((entry) =>
        entry.principalId === ADMIN ? member(ADMIN, ['view', 'manage']) : entry,
      ),
    };
    expect(assignableRoles(silent, ADMIN, of(VIEWER))).toEqual([]);
    expect(assignableRoles(silent, ADMIN, of(MEMBER))).toEqual(['member', 'viewer']);
  });

  it('设成管理员：加上管理、邀请；他的 Agent 能不能说话照旧，原来只能看的按默认开', () => {
    const all = grantableBy(room, OWNER);
    expect(permissionsForRole(permissions('view', 'speak'), 'admin', all).capabilities).toEqual([
      'view',
      'speak',
      'invite',
      'manage',
    ]);
    expect(
      permissionsForRole(permissions('view', 'speak', 'automate'), 'admin', all).capabilities,
    ).toEqual(['view', 'speak', 'invite', 'manage', 'automate']);
    expect(permissionsForRole(permissions('view'), 'admin', all).capabilities).toEqual([
      'view',
      'speak',
      'invite',
      'manage',
      'automate',
    ]);
  });

  it('改成成员：去掉管理，能不能邀请照旧；原来只能看的，他的 Agent 按默认能说话', () => {
    const all = grantableBy(room, OWNER);
    expect(
      permissionsForRole(permissions('view', 'speak', 'invite', 'manage'), 'member', all)
        .capabilities,
    ).toEqual(['view', 'speak', 'invite']);
    expect(permissionsForRole(permissions('view'), 'member', all).capabilities).toEqual([
      'view',
      'speak',
      'automate',
    ]);
  });

  it('改成只能看：只剩查看', () => {
    expect(
      permissionsForRole(
        permissions('view', 'speak', 'invite', 'manage', 'automate'),
        'viewer',
        grantableBy(room, OWNER),
      ).capabilities,
    ).toEqual(['view']);
  });

  it('自己给不出的不加：没有自动发言的管理员把只能看的人改回成员，只给发言', () => {
    // ADMIN 有查看、发言、邀请、管理，没有自动发言。
    expect(
      permissionsForRole(permissions('view'), 'member', grantableBy(room, ADMIN)).capabilities,
    ).toEqual(['view', 'speak']);
  });

  it('管理员自己不当了：只去掉管理', () => {
    expect(
      permissionsForRole(of(ADMIN).permissions, 'member', grantableBy(room, ADMIN)).capabilities,
    ).toEqual(['view', 'speak', 'invite']);
  });

  it('设、撤管理员才算跨过管理员那条线', () => {
    expect(crossesAdmin('member', 'admin')).toBe(true);
    expect(crossesAdmin('viewer', 'admin')).toBe(true);
    expect(crossesAdmin('admin', 'member')).toBe(true);
    expect(crossesAdmin('admin', 'viewer')).toBe(true);
    expect(crossesAdmin('member', 'viewer')).toBe(false);
    expect(crossesAdmin('viewer', 'member')).toBe(false);
    expect(crossesAdmin('admin', 'admin')).toBe(false);
  });

  it('两个开关和移出、封禁：房主动得了别人，管理员动得了不是管理员的，自己那一行都动不了', () => {
    expect(canManageMember(room, OWNER, of(ADMIN))).toBe(true);
    expect(canManageMember(room, OWNER, of(OTHER_ADMIN))).toBe(true);
    expect(canManageMember(room, OWNER, of(OWNER))).toBe(false);
    expect(canManageMember(room, OWNER, of(BANNED))).toBe(false);
    expect(canManageMember(room, ADMIN, of(MEMBER))).toBe(true);
    expect(canManageMember(room, ADMIN, of(VIEWER))).toBe(true);
    expect(canManageMember(room, ADMIN, of(OTHER_ADMIN))).toBe(false);
    expect(canManageMember(room, ADMIN, of(OWNER))).toBe(false);
    expect(canManageMember(room, ADMIN, of(ADMIN))).toBe(false);
    expect(canManageMember(room, MEMBER, of(VIEWER))).toBe(false);
    expect(canManageMember({ ...room, status: 'archived' }, OWNER, of(MEMBER))).toBe(false);
  });

  it('邀请时给不出“管理”，房主也一样', () => {
    expect(invitableBy(room, OWNER).capabilities).toEqual(['view', 'speak', 'invite', 'automate']);
    expect(invitableBy(room, ADMIN).capabilities).toEqual(['view', 'speak', 'invite']);
    expect(invitableBy(room, VIEWER).capabilities).toEqual(['view']);
    expect(invitableBy(room, BANNED).capabilities).toEqual([]);
  });
});
