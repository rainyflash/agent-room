import { z } from 'zod';

import type { Result } from '@/shared/result';

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const matrixRoomIdSchema = z
  .string()
  .min(4)
  .max(255)
  .regex(/^![^:]+:[^:]+$/u);
const matrixUserIdSchema = z
  .string()
  .min(4)
  .max(255)
  .regex(/^@[^:]+:[^:]+$/u);

export const directAgentSchema = z
  .object({
    agentId: uuidV7Schema,
    avatarContentId: uuidV7Schema.nullable(),
    displayName: z.string().min(1).max(80),
    matrixUserId: matrixUserIdSchema,
  })
  .strict();

export const directContactPolicySchema = z
  .object({
    agentBlocksPrincipal: z.boolean(),
    deliveryAllowed: z.boolean(),
    presenceDisclosure: z.enum(['coarse', 'hidden']),
    principalBlocksAgent: z.boolean(),
  })
  .strict()
  .superRefine((policy, context) => {
    const deliveryAllowed = !policy.agentBlocksPrincipal && !policy.principalBlocksAgent;
    const disclosure = deliveryAllowed ? 'coarse' : 'hidden';
    if (policy.deliveryAllowed !== deliveryAllowed || policy.presenceDisclosure !== disclosure) {
      context.addIssue({ code: 'custom', message: '联系策略投影互相矛盾。' });
    }
  });

export const directSessionSchema = z
  .object({
    catalogId: uuidV7Schema,
    contactPolicy: directContactPolicySchema,
    lifecycle: z.enum(['provisioning', 'active', 'failed']),
    matrixRoomId: matrixRoomIdSchema.nullable(),
    roomInstanceId: uuidV7Schema.nullable(),
    target: directAgentSchema,
    version: z.number().int().min(0),
  })
  .strict()
  .superRefine((session, context) => {
    const active = session.lifecycle === 'active';
    if (active !== (session.matrixRoomId !== null && session.roomInstanceId !== null)) {
      context.addIssue({ code: 'custom', message: '直接会话生命周期与房间实例投影不一致。' });
    }
  });

export const directSessionListSchema = z
  .object({ sessions: z.array(directSessionSchema).max(500) })
  .strict();

export const directContactSchema = z
  .object({
    contactPolicy: directContactPolicySchema,
    target: directAgentSchema,
  })
  .strict();

export type DirectAgent = z.output<typeof directAgentSchema>;
export type DirectContactPolicy = z.output<typeof directContactPolicySchema>;
export type DirectSession = z.output<typeof directSessionSchema>;
export type DirectContact = z.output<typeof directContactSchema>;

export type DirectSessionFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

export type DirectSessionGateway = {
  inspect(catalogId: string): Promise<Result<DirectSession, DirectSessionFailure>>;
  list(): Promise<Result<readonly DirectSession[], DirectSessionFailure>>;
  open(targetAgentId: string): Promise<Result<DirectSession, DirectSessionFailure>>;
  setBlocked(
    targetAgentId: string,
    blocked: boolean,
  ): Promise<Result<DirectContact, DirectSessionFailure>>;
};

export type DirectSessionMatrixGateway = {
  markDisplayed(roomId: string, matrixEventId: string): Promise<Result<void, DirectSessionFailure>>;
  prepare(session: DirectSession): Promise<Result<void, DirectSessionFailure>>;
  setIgnored(matrixUserId: string, ignored: boolean): Promise<Result<void, DirectSessionFailure>>;
};
