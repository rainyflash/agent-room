import { describe, expect, it } from 'vitest';

import { permissions, type PrivateRoomCapability } from './private-room';
import {
  roleChangeFacts,
  roomRoleChangesSchema,
  type RoomRoleChange,
  type RoomStanding,
} from './room-role-changes';

const OWNER = '0198b601-77a1-7bb8-83eb-a8fe68c97e42';
const GRACE = '0198b601-77a1-7bb8-83eb-a8fe68c97e43';

const ADMIN: PrivateRoomCapability[] = ['view', 'speak', 'invite', 'manage'];
const MEMBER: PrivateRoomCapability[] = ['view', 'speak', 'automate'];

function standing(
  capabilities: PrivateRoomCapability[],
  status: RoomStanding['status'] = 'joined',
  owner = false,
): RoomStanding {
  return { owner, permissions: permissions(...capabilities), status };
}

function change(
  previous: RoomStanding | undefined,
  next: RoomStanding,
  // null 是没有动手的人（给 undefined 会落回默认的房主）。
  actorPrincipalId: string | null = OWNER,
): RoomRoleChange {
  return {
    ...(actorPrincipalId === null ? {} : { actorPrincipalId }),
    changedAtUnixMs: 1_700_000_000_000,
    id: 1,
    next,
    ...(previous === undefined ? {} : { previous }),
    targetPrincipalId: GRACE,
  };
}

describe('身份变动记录说的事', () => {
  it('在房间里换了身份：设、撤管理员，改成只能看，转让房主的两头', () => {
    expect(roleChangeFacts(change(standing(MEMBER), standing(ADMIN)))).toEqual([
      { kind: 'role', from: 'member', to: 'admin' },
    ]);
    expect(roleChangeFacts(change(standing(ADMIN), standing(['view'])))).toEqual([
      { kind: 'role', from: 'admin', to: 'viewer' },
    ]);
    const all: PrivateRoomCapability[] = ['view', 'speak', 'invite', 'manage', 'automate'];
    expect(roleChangeFacts(change(standing(all, 'joined', true), standing(MEMBER)))).toEqual([
      { kind: 'role', from: 'owner', to: 'member' },
    ]);
    expect(roleChangeFacts(change(standing(MEMBER), standing(all, 'joined', true)))).toEqual([
      { kind: 'role', from: 'member', to: 'owner' },
    ]);
  });

  it('身份没变、拨了开关：拨了哪个说哪个，一次拨两个就是两件', () => {
    expect(
      roleChangeFacts(change(standing(MEMBER), standing(['view', 'speak', 'invite', 'automate']))),
    ).toEqual([{ kind: 'switch', capability: 'invite', on: true }]);
    expect(roleChangeFacts(change(standing(MEMBER), standing(['view', 'speak'])))).toEqual([
      { kind: 'switch', capability: 'automate', on: false },
    ]);
    expect(
      roleChangeFacts(change(standing(MEMBER), standing(['view', 'speak', 'invite']))),
    ).toEqual([
      { kind: 'switch', capability: 'invite', on: true },
      { kind: 'switch', capability: 'automate', on: false },
    ]);
    // 管理员没了“发言”：身份和两个开关都没变，只说权限改了。
    expect(
      roleChangeFacts(change(standing(ADMIN), standing(['view', 'invite', 'manage']))),
    ).toEqual([{ kind: 'permissions' }]);
  });

  it('请进来当管理员：之前不在名单里的、之前被移出的都算进来', () => {
    expect(roleChangeFacts(change(undefined, standing(ADMIN, 'invited')))).toEqual([
      { kind: 'entered', as: 'admin' },
    ]);
    expect(roleChangeFacts(change(standing([], 'removed'), standing(ADMIN, 'invited')))).toEqual([
      { kind: 'entered', as: 'admin' },
    ]);
  });

  it('管理员不在房间里了：自己走的、被移出的、被封禁的、没接受邀请的分开说', () => {
    const gone = (status: RoomStanding['status']) => standing([], status);
    expect(roleChangeFacts(change(standing(ADMIN), gone('removed'), GRACE))).toEqual([
      { kind: 'exited', from: 'admin', how: 'left' },
    ]);
    expect(roleChangeFacts(change(standing(ADMIN), gone('removed')))).toEqual([
      { kind: 'exited', from: 'admin', how: 'removed' },
    ]);
    expect(roleChangeFacts(change(standing(ADMIN), gone('banned')))).toEqual([
      { kind: 'exited', from: 'admin', how: 'banned' },
    ]);
    expect(roleChangeFacts(change(standing(ADMIN, 'invited'), gone('declined'), GRACE))).toEqual([
      { kind: 'exited', from: 'admin', how: 'declined' },
    ]);
    // 不是谁动的手（以后房主删除账户）：照被移出说。
    expect(roleChangeFacts(change(standing(ADMIN), gone('removed'), null))).toEqual([
      { kind: 'exited', from: 'admin', how: 'removed' },
    ]);
  });
});

describe('身份变动记录的回答', () => {
  it('没有动手的人、没有之前的都读得出来；以后多出来的字段不挡着读', () => {
    const parsed = roomRoleChangesSchema.parse({
      changes: [
        {
          changedAtUnixMs: 1_700_000_000_000,
          id: 12,
          next: { owner: false, permissions: { capabilities: ADMIN }, status: 'invited' },
          subRoomCatalogId: '0198b601-77a1-7bb8-83eb-a8fe68c97e99',
          targetPrincipalId: GRACE,
        },
      ],
      nextCursor: 'later',
    });

    expect(parsed.changes).toHaveLength(1);
    expect(parsed.changes[0]?.actorPrincipalId).toBeUndefined();
    expect(parsed.changes[0]?.previous).toBeUndefined();
    expect(parsed.changes[0]?.next.permissions.capabilities).toEqual(ADMIN);
  });

  it('状态不认识、少了对象的不收', () => {
    const entry = {
      changedAtUnixMs: 1,
      id: 1,
      next: { owner: false, permissions: { capabilities: [] }, status: 'joined' },
      targetPrincipalId: GRACE,
    };
    expect(
      roomRoleChangesSchema.safeParse({
        changes: [{ ...entry, next: { ...entry.next, status: 'left' } }],
      }).success,
    ).toBe(false);
    expect(
      roomRoleChangesSchema.safeParse({ changes: [{ ...entry, targetPrincipalId: undefined }] })
        .success,
    ).toBe(false);
  });
});
