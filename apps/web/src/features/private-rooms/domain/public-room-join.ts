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

export type PendingJoinRequest = z.infer<typeof pendingJoinRequestSchema>;
export type JoinRequestDecision = 'approve' | 'reject';

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
