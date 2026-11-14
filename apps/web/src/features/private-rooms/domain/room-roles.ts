import {
  grantableBy,
  memberFor,
  privateRoomCapabilities,
  type PrivateRoom,
  type PrivateRoomCapability,
  type PrivateRoomMember,
  type PrivateRoomPermissions,
} from './private-room';

/**
 * 房间里的身份（`specs/redesign/room-roles.md`）：五项权限上面的一层名字。服务器存的、接口传的还是那五项，
 * 身份由网页照权限算出来。
 */
export const roomRoles = ['owner', 'admin', 'member', 'viewer'] as const;
export type RoomRole = (typeof roomRoles)[number];
/** 选得出来的身份：房主只能转让，不在里面。 */
export type AssignableRole = Exclude<RoomRole, 'owner'>;

/** 这个人此刻是什么身份。只对还在房间里的人（受邀、已加入）有意义。 */
export function roomRole(room: PrivateRoom, member: PrivateRoomMember): RoomRole {
  return member.principalId === room.ownerPrincipalId ? 'owner' : roleOf(member.permissions);
}

/** 不是房主的人，照这份权限算是什么身份。 */
export function roleOf(permissions: PrivateRoomPermissions): AssignableRole {
  if (permissions.capabilities.includes('manage')) return 'admin';
  return permissions.capabilities.includes('speak') ? 'member' : 'viewer';
}

function activeMember(member: PrivateRoomMember): boolean {
  return member.status === 'invited' || member.status === 'joined';
}

/**
 * 看的人能把这一行换成哪几种身份，现在的那一种也在里面。少于两种就是换不了，界面只写字、不给选择框。
 * 和服务器一样：
 * - 房主能把别人设成管理员、成员、只能看；
 * - 管理员动不了房主和别的管理员，成员在“成员”和“只能看”之间换；
 * - 管理员自己可以不当了，降成成员，回不来。
 */
export function assignableRoles(
  room: PrivateRoom,
  actorPrincipalId: string,
  target: PrivateRoomMember,
): readonly AssignableRole[] {
  const current = roomRole(room, target);
  if (current === 'owner' || !activeMember(target) || room.status !== 'active') return [];
  if (actorPrincipalId === room.ownerPrincipalId) return ['admin', 'member', 'viewer'];
  const actor = memberFor(room, actorPrincipalId);
  if (actor?.status !== 'joined' || !actor.permissions.capabilities.includes('manage')) return [];
  if (target.principalId === actorPrincipalId) return ['admin', 'member'];
  if (current === 'admin') return [];
  // 把只能看的人改回成员要给出“发言”，自己没有就给不出。
  return current === 'member' || actor.permissions.capabilities.includes('speak')
    ? ['member', 'viewer']
    : [];
}

/**
 * 换成这个身份以后的权限。
 * - 管理员：加上“管理”，也加上查看、发言和邀请（管理员能请人）；他的 Agent 能不能说话照旧，
 *   原来只能看的按默认开着。
 * - 成员：去掉“管理”，查看和发言都有；原来只能看的，他的 Agent 按默认能说话。能不能邀请照旧。
 * - 只能看：只剩查看。
 *
 * `grantable` 是操作的人给得出的（`grantableBy`）：给不出的那几项不加，免得整次被服务器拒掉。
 */
export function permissionsForRole(
  current: PrivateRoomPermissions,
  role: AssignableRole,
  grantable: PrivateRoomPermissions,
): PrivateRoomPermissions {
  if (role === 'viewer') return ordered(['view']);
  const had = new Set<PrivateRoomCapability>(current.capabilities);
  const next = new Set<PrivateRoomCapability>(had);
  const add = (capability: PrivateRoomCapability): void => {
    if (had.has(capability) || grantable.capabilities.includes(capability)) next.add(capability);
  };
  add('view');
  add('speak');
  if (!had.has('speak')) add('automate');
  if (role === 'admin') {
    add('invite');
    add('manage');
  } else {
    next.delete('manage');
  }
  if (!next.has('speak')) next.delete('automate');
  return ordered([...next]);
}

/** 这次换身份是不是设、撤管理员：这两样要最近登录过（自己不当了除外）。 */
export function crossesAdmin(from: RoomRole, to: AssignableRole): boolean {
  return (from === 'admin') !== (to === 'admin');
}

/**
 * 看的人动得了这一行的两个开关（他的 Agent 能说话、能邀请人）和移出、封禁：
 * 房主动得了别人，管理员动得了不是管理员的人；自己那一行谁都动不了。
 */
export function canManageMember(
  room: PrivateRoom,
  actorPrincipalId: string,
  target: PrivateRoomMember,
): boolean {
  if (room.status !== 'active' || !activeMember(target)) return false;
  if (target.principalId === actorPrincipalId || target.principalId === room.ownerPrincipalId)
    return false;
  if (actorPrincipalId === room.ownerPrincipalId) return true;
  const actor = memberFor(room, actorPrincipalId);
  return (
    actor?.status === 'joined' &&
    actor.permissions.capabilities.includes('manage') &&
    !target.permissions.capabilities.includes('manage')
  );
}

/**
 * 邀请人、发邀请链接时给得出的权限：自己有的，去掉“管理”。房主也一样：要设管理员，等人在名单里了
 * 到成员一节里选（那里要最近登录过）。一条谁点了都是管理员的链接不该一勾就有。
 */
export function invitableBy(room: PrivateRoom, principalId: string): PrivateRoomPermissions {
  return ordered(
    grantableBy(room, principalId).capabilities.filter((capability) => capability !== 'manage'),
  );
}

function ordered(capabilities: readonly PrivateRoomCapability[]): PrivateRoomPermissions {
  return {
    capabilities: Object.freeze(
      privateRoomCapabilities.filter((capability) => capabilities.includes(capability)),
    ),
  };
}
