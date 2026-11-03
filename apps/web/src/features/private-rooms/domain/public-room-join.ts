import { z } from 'zod';

import type { Result } from '@/shared/result';
import { privateRoomPrincipalIdSchema, type PrivateRoomFailure } from './private-room';

/**
 * 申请加入公开房间（`specs/public-rooms/design.md`“加入方式”）：房主和管理员看在等的申请，批准或拒绝。
 * 申请一直等着，没人处理也不作废；改回私密、归档时作废。批准以后申请人的网页自动加入。
 */
const unixMillisSchema = z.number().int().nonnegative();

const pendingJoinRequestSchema = z
  .object({
    displayName: z.string(),
    /** 申请时留的一句话，最多 200 字，可以换行；没留是空的。 */
    message: z.string(),
    principalId: privateRoomPrincipalIdSchema,
    requestedAtUnixMs: unixMillisSchema,
  })
  .strict();

export const pendingJoinRequestsSchema = z
  .object({ requests: z.array(pendingJoinRequestSchema) })
  .strict();

export const answeredJoinRequestSchema = z
  .object({ status: z.enum(['approved', 'rejected']) })
  .strict();

/** 申请留言最多 200 字（按字算，和服务器一样），可以换行。 */
export const JOIN_REQUEST_MESSAGE_MAX_LENGTH = 200;

/**
 * 直接进、申请以后你在这间房里的处境：`pending` 在等批准，`invited` 服务器以房主的名义邀请了你、网页
 * 接着照接受邀请那一套在 Matrix 上加入，`joined` 已经在里面。
 */
export const publicRoomMembershipSchema = z
  .object({
    catalogId: z.string().min(1),
    membership: z.enum(['pending', 'invited', 'joined']),
    requestedAtUnixMs: unixMillisSchema.optional(),
  })
  .strict();

export type PendingJoinRequest = z.infer<typeof pendingJoinRequestSchema>;
export type PublicRoomMembership = z.infer<typeof publicRoomMembershipSchema>;
export type JoinRequestDecision = 'approve' | 'reject';

/** 从发现页进公开房间：直接进、申请、撤回自己的申请。 */
export type PublicRoomEntryGateway = {
  /** Joins a room anyone can join directly; a room that needs approval is `room_join.approval_required`. */
  join(catalogId: string): Promise<Result<PublicRoomMembership, PrivateRoomFailure>>;
  /**
   * Asks to join, optionally with a message of up to 200 characters. A room anyone can join answers
   * `invited` straight away; asking again while waiting keeps the first request.
   */
  requestToJoin(
    catalogId: string,
    message: string,
  ): Promise<Result<PublicRoomMembership, PrivateRoomFailure>>;
  /** Withdraws your own request that is still waiting. */
  withdrawJoinRequest(catalogId: string): Promise<Result<void, PrivateRoomFailure>>;
};

export type PublicRoomJoinGateway = {
  /** Requests waiting for an answer, oldest first; the owner and admins can ask. */
  joinRequests(
    catalogId: string,
  ): Promise<Result<readonly PendingJoinRequest[], PrivateRoomFailure>>;
  /**
   * Approves or rejects one person's request. Approving invites them in the owner's name; a banned
   * person has to be unbanned first. Someone rejected can ask again after 24 hours.
   */
  answerJoinRequest(
    catalogId: string,
    principalId: string,
    decision: JoinRequestDecision,
  ): Promise<Result<void, PrivateRoomFailure>>;
};
