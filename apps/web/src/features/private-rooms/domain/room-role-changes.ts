import { z } from 'zod';

import type { Result } from '@/shared/result';

import {
  privateRoomPermissionsSchema,
  privateRoomPrincipalIdSchema,
  type PrivateRoomFailure,
} from './private-room';
import { roleOf, type RoomRole } from './room-roles';

/** 一个人在房间里的位置：成员状态、是不是房主、身上的权限。身份是照它算出来的。 */
const standingSchema = z.object({
  owner: z.boolean(),
  permissions: privateRoomPermissionsSchema,
  status: z.enum(['invited', 'joined', 'declined', 'removed', 'banned']),
});

/**
 * 身份变动记录（`specs/redesign/room-roles.md` 的“记录”）：谁、什么时候、把谁、从什么改成了什么，新的在前。
 * 只有房主和管理员问得到。
 *
 * 读父房间的时候连它下面子房间里有人动手的一起给，写着是哪一间（`subRoom`，R5）。
 *
 * 这里故意不 `.strict()`：服务器以后再加字段，已经装着的应用得读得过去。
 */
export const roomRoleChangesSchema = z.object({
  changes: z.array(
    z.object({
      /** 不是谁动的手时没有。 */
      actorPrincipalId: privateRoomPrincipalIdSchema.optional(),
      changedAtUnixMs: z.number().int().nonnegative(),
      id: z.number().int(),
      next: standingSchema,
      /** 之前名单里没有这个人时没有。 */
      previous: standingSchema.optional(),
      targetPrincipalId: privateRoomPrincipalIdSchema,
      /** 是这间房下面哪一间子房间里的；这间房自己的没有。名字是此刻的。 */
      subRoom: z.object({ catalogId: z.string().min(1), name: z.string() }).optional(),
    }),
  ),
});

export type RoomRoleChange = z.infer<typeof roomRoleChangesSchema>['changes'][number];
export type RoomStanding = RoomRoleChange['next'];

export type RoomRoleChangeGateway = {
  /** The room's latest role changes, newest first; only the owner and admins can ask. */
  roleChanges(catalogId: string): Promise<Result<readonly RoomRoleChange[], PrivateRoomFailure>>;
};

/** 怎么不在房间里了。自己走的和被移出的，服务器记的是同一种状态，看是不是他自己动的手。 */
export type RoomExit = 'banned' | 'declined' | 'left' | 'removed';

/** 一条记录说的事。一条多半只有一件；一次改了两个开关就是两件。 */
export type RoleChangeFact =
  /** 在房间里换了身份。转让房主是两条记录：原房主一条，接手的人一条。 */
  | { readonly kind: 'role'; readonly from: RoomRole; readonly to: RoomRole }
  /** 请进来就是房主或者管理员。 */
  | { readonly kind: 'entered'; readonly as: RoomRole }
  /** 房主或者管理员不在房间里了。 */
  | { readonly kind: 'exited'; readonly from: RoomRole; readonly how: RoomExit }
  /** 身份没变，拨了“更多”里的开关。 */
  | { readonly kind: 'switch'; readonly capability: 'automate' | 'invite'; readonly on: boolean }
  /** 身份和两个开关都没变，别的权限改了（只有直接调接口改得出来）。 */
  | { readonly kind: 'permissions' };

function active(standing: RoomStanding | undefined): standing is RoomStanding {
  return standing?.status === 'invited' || standing?.status === 'joined';
}

function roleIn(standing: RoomStanding): RoomRole {
  return standing.owner ? 'owner' : roleOf(standing.permissions);
}

/** 这条记录说了哪几件事。服务器只记值得记的变动，所以至少有一件。 */
export function roleChangeFacts(change: RoomRoleChange): readonly RoleChangeFact[] {
  const { next, previous } = change;
  if (!active(next)) {
    // 之前也不在房间里的服务器不记；真碰上了就照“权限改了”说。
    if (!active(previous)) return [{ kind: 'permissions' }];
    return [{ kind: 'exited', from: roleIn(previous), how: exitOf(change) }];
  }
  if (!active(previous)) return [{ kind: 'entered', as: roleIn(next) }];
  const from = roleIn(previous);
  const to = roleIn(next);
  if (from !== to) return [{ kind: 'role', from, to }];
  const switches = (['invite', 'automate'] as const).flatMap((capability): RoleChangeFact[] => {
    const had = previous.permissions.capabilities.includes(capability);
    const has = next.permissions.capabilities.includes(capability);
    return had === has ? [] : [{ kind: 'switch', capability, on: has }];
  });
  return switches.length > 0 ? switches : [{ kind: 'permissions' }];
}

function exitOf(change: RoomRoleChange): RoomExit {
  switch (change.next.status) {
    case 'banned':
      return 'banned';
    case 'declined':
      return 'declined';
    default:
      return change.actorPrincipalId === change.targetPrincipalId ? 'left' : 'removed';
  }
}
