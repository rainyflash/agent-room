import { z } from 'zod';

import type { Result } from '@/shared/result';
import {
  privateRoomCapabilities,
  privateRoomPermissionsSchema,
  type PrivateRoomFailure,
  type PrivateRoomPermissions,
} from './private-room';

/**
 * 房间的公开设置（`specs/public-rooms/design.md`）：进不进发现页、别人怎么进、新成员拿到什么。
 * 房间里已加入的人都读得到（公开房间的成员都能分享），只有房主能改。单独一个接口，不加进
 * `privateRoomSchema`：旧版桌面端对它严格校验，多一个字段整个房间都读不出来。
 */
export const roomJoinModes = ['open', 'request', 'invite_only'] as const;

export type RoomJoinMode = (typeof roomJoinModes)[number];

const unixMillisSchema = z.number().int().nonnegative();

/** 6 位大小写字母和数字，第一次公开时生成；换码以后旧的马上作废。 */
const shareCodePattern = /^[A-Za-z0-9]{6}$/u;

export const privateRoomListingSchema = z
  .object({
    allowNetworkAgents: z.boolean(),
    endToEndEncrypted: z.boolean(),
    firstListedAtUnixMs: unixMillisSchema.nullable(),
    joinMode: z.enum(roomJoinModes),
    listed: z.boolean(),
    memberPermissions: privateRoomPermissionsSchema,
    /** 平台把房间从发现页拿掉了：原因和时间。 */
    platformHold: z
      .object({ heldAtUnixMs: unixMillisSchema, reason: z.string() })
      .strict()
      .nullable(),
    shareCode: z.string().regex(shareCodePattern).nullable(),
    /** 服务器拼好的整条分享链接（裸域上的 `/r/<分享码>`）：网页不知道裸域在哪。 */
    shareUrl: z.url({ protocol: /^https?$/u }).nullable(),
  })
  .strict();

export type PrivateRoomListing = z.infer<typeof privateRoomListingSchema>;

/** 房主改的那几项。 */
export type PrivateRoomListingSettings = Pick<
  PrivateRoomListing,
  'allowNetworkAgents' | 'joinMode' | 'listed' | 'memberPermissions'
>;

export type PrivateRoomListingGateway = {
  /** The room's public settings; any joined member can read them. */
  listing(catalogId: string): Promise<Result<PrivateRoomListing, PrivateRoomFailure>>;
  /**
   * Only the owner may change them. Fails with `room_listing.limit_reached` when the owner already has
   * another public room.
   */
  updateListing(
    catalogId: string,
    settings: PrivateRoomListingSettings,
  ): Promise<Result<PrivateRoomListing, PrivateRoomFailure>>;
  /** The owner gives the room a new share code; the old link stops working at once. */
  changeShareCode(catalogId: string): Promise<Result<PrivateRoomListing, PrivateRoomFailure>>;
};

/**
 * 没有账号的网络 Agent 拿房间号来时这间房怎么进，和控制面的 `knock_entry` 一样：私密的照旧敲门；公开的，
 * 只能邀请的、关了没有账号的网络 Agent 的只认口令，要申请的、被平台从发现页拿掉的敲门等放行，别的直接进。
 */
export type NetworkAgentEntry = 'code' | 'knock' | 'open';

export function networkAgentEntry(listing: PrivateRoomListing | null): NetworkAgentEntry {
  if (listing?.listed !== true) return 'knock';
  if (!listing.allowNetworkAgents || listing.joinMode === 'invite_only') return 'code';
  return listing.joinMode === 'request' || listing.platformHold !== null ? 'knock' : 'open';
}

/** 新成员能拿到的权限：一定能查看，不能带着管理（和服务器同一条规则）。 */
export const grantableToNewMembers: PrivateRoomPermissions = {
  capabilities: privateRoomCapabilities.filter((capability) => capability !== 'manage'),
};

export function settingsOf(listing: PrivateRoomListing): PrivateRoomListingSettings {
  return {
    allowNetworkAgents: listing.allowNetworkAgents,
    joinMode: listing.joinMode,
    listed: listing.listed,
    memberPermissions: listing.memberPermissions,
  };
}

export function sameSettings(
  left: PrivateRoomListingSettings,
  right: PrivateRoomListingSettings,
): boolean {
  return (
    left.allowNetworkAgents === right.allowNetworkAgents &&
    left.joinMode === right.joinMode &&
    left.listed === right.listed &&
    privateRoomCapabilities.every(
      (capability) =>
        left.memberPermissions.capabilities.includes(capability) ===
        right.memberPermissions.capabilities.includes(capability),
    )
  );
}

/** 别人拿着分享链接进得来才给分享：只能邀请的房间链接打开了也进不来。 */
export function shareable(listing: PrivateRoomListing): listing is PrivateRoomListing & {
  readonly shareUrl: string;
} {
  return (
    listing.listed &&
    listing.platformHold === null &&
    listing.joinMode !== 'invite_only' &&
    listing.shareUrl !== null
  );
}
