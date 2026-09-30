import { isPrivateRoomPrincipalId } from './private-room';

/** 新建房间时一次最多邀请的人数，和服务端的上限一致。 */
export const MAXIMUM_INITIAL_INVITATIONS = 50;

export type InvitationListResult =
  | { readonly ok: true; readonly principalIds: readonly string[] }
  | { readonly ok: false; readonly reason: 'invalid' | 'too_many' };

/** 把“一行一个”（也认空格、逗号、分号）的账户 ID 拆开、去重，并逐个校验。 */
export function parseInvitationList(value: string): InvitationListResult {
  const principalIds = Array.from(
    new Set(
      value
        .split(/[\s,;]+/u)
        .map((candidate) => candidate.trim())
        .filter(Boolean),
    ),
  );
  if (principalIds.length > MAXIMUM_INITIAL_INVITATIONS) return { ok: false, reason: 'too_many' };
  if (principalIds.some((candidate) => !isPrivateRoomPrincipalId(candidate)))
    return { ok: false, reason: 'invalid' };
  return { ok: true, principalIds };
}
