import { z } from 'zod';

import type { Result } from '@/shared/result';
import {
  permissions,
  privateRoomPermissionsSchema,
  type PrivateRoomFailure,
  type PrivateRoomPermissions,
} from './private-room';

/**
 * 私人房间的邀请链接（`specs/room-people/design.md`“邀请链接”）：一个房间同时只有一条，7 天后过期，
 * 有效期内谁拿到都能用。秘密放在 `/join#` 后面，浏览器不会把这一段发给服务器，也不进 Referer。
 */
const secretPattern = /^[0-9a-hjkmnp-tv-z]{26}$/u;

/** 链接里的秘密：26 个小写 Crockford Base32 字符；大写和首尾空白照收。不像的返回空。 */
export function parseInviteSecret(value: string): string | null {
  const secret = value.trim().toLowerCase();
  return secretPattern.test(secret) ? secret : null;
}

const unixMillisSchema = z.number().int().nonnegative();
const membershipSchema = z.enum(['absent', 'invited', 'joined']);

export const privateRoomInviteLinkSchema = z
  .object({
    createdAtUnixMs: unixMillisSchema,
    expiresAtUnixMs: unixMillisSchema,
    permissions: privateRoomPermissionsSchema,
    secret: z.string().regex(secretPattern),
    /** 服务器按部署的网页地址拼好的：桌面端页面的 Origin 不是网页的，拼不出别人打得开的链接。 */
    url: z.url({ protocol: /^https?$/u }),
  })
  .strict();

export type PrivateRoomInviteLink = z.infer<typeof privateRoomInviteLinkSchema>;

export const privateRoomInvitePreviewSchema = z
  .object({
    catalogId: z.uuid(),
    expiresAtUnixMs: unixMillisSchema,
    inviterDisplayName: z.string(),
    /** 打开链接的人此刻在不在这个房间：没有、受邀还没答复、已经加入。 */
    membership: membershipSchema,
    permissions: privateRoomPermissionsSchema,
    roomName: z.string(),
  })
  .strict();

export type PrivateRoomInvitePreview = z.infer<typeof privateRoomInvitePreviewSchema>;

export const redeemedPrivateRoomInviteSchema = z
  .object({ catalogId: z.uuid(), membership: membershipSchema })
  .strict();

export type RedeemedPrivateRoomInvite = z.infer<typeof redeemedPrivateRoomInviteSchema>;

/** 链接和按账户 ID 邀请默认都给查看、发言、自动发言：被邀请的人带进来的 Agent 也能说话。 */
export const DEFAULT_INVITE_PERMISSIONS: PrivateRoomPermissions = permissions(
  'view',
  'speak',
  'automate',
);

/** 管链接要“邀请”权限；打开和兑换要登录。都和房间的其它请求一样带着这台设备的登录。 */
export type PrivateRoomInviteGateway = {
  /** 再看现在这条；没有时 `invite_link.not_found`，看不到时 `invite_link.not_revealable`。 */
  inviteLink(catalogId: string): Promise<Result<PrivateRoomInviteLink, PrivateRoomFailure>>;
  /** 生成或换一条，旧的马上不能用。不给权限就按默认的，服务器再按你自己有的截一下。 */
  createInviteLink(
    catalogId: string,
    permissions?: PrivateRoomPermissions,
  ): Promise<Result<PrivateRoomInviteLink, PrivateRoomFailure>>;
  disableInviteLink(catalogId: string): Promise<Result<void, PrivateRoomFailure>>;
  /** 打开链接的人看到的：哪个房间、谁邀请的、会给什么权限。 */
  previewInvite(secret: string): Promise<Result<PrivateRoomInvitePreview, PrivateRoomFailure>>;
  /** 以链接创建人的名义邀请；已经受邀或加入的不改原来的邀请。之后照接受邀请那一套进房间。 */
  redeemInvite(secret: string): Promise<Result<RedeemedPrivateRoomInvite, PrivateRoomFailure>>;
};
