import { z } from 'zod';

import type { Result } from '@/shared/result';

export const automationMessageKinds = ['room_message', 'reply'] as const;
export const automationAudiences = ['known_room_members', 'any_room_member'] as const;
export const automationGrantStatuses = ['active', 'revoked', 'exhausted', 'expired'] as const;

export type AutomationMessageKind = (typeof automationMessageKinds)[number];
export type AutomationAudience = (typeof automationAudiences)[number];

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const timestampSchema = z.number().int().nonnegative();

export const automationGrantSchema = z
  .object({
    agentId: uuidV7Schema,
    agentInstanceId: uuidV7Schema.nullable(),
    audience: z.enum(automationAudiences),
    expiresAtUnixMs: timestampSchema,
    grantId: uuidV7Schema,
    maxMessagesPerMinute: z.number().int().min(1).max(60),
    maxTotalMessages: z.number().int().min(1).max(10_000).nullable(),
    messageKinds: z.array(z.enum(automationMessageKinds)).min(1).max(2),
    messagesInCurrentMinute: z.number().int().nonnegative(),
    requiresRiskScan: z.boolean(),
    revokedAtUnixMs: timestampSchema.nullable(),
    roomCatalogId: uuidV7Schema,
    startsAtUnixMs: timestampSchema,
    status: z.enum(automationGrantStatuses),
    totalMessages: z.number().int().nonnegative(),
  })
  .strict()
  .superRefine((grant, context) => {
    if (grant.expiresAtUnixMs <= grant.startsAtUnixMs) {
      context.addIssue({ code: 'custom', message: '授权时间窗口无效。' });
    }
    if ((grant.status === 'revoked') !== (grant.revokedAtUnixMs !== null)) {
      context.addIssue({ code: 'custom', message: '授权撤销状态与时间互相矛盾。' });
    }
    if (grant.messagesInCurrentMinute > grant.maxMessagesPerMinute) {
      context.addIssue({ code: 'custom', message: '分钟用量超过授权上限。' });
    }
    if (grant.maxTotalMessages !== null && grant.totalMessages > grant.maxTotalMessages) {
      context.addIssue({ code: 'custom', message: '累计用量超过授权上限。' });
    }
  });

export const automationGrantListSchema = z
  .object({ grants: z.array(automationGrantSchema).max(10_000) })
  .strict();

export const createAutomationGrantInputSchema = z
  .object({
    agentId: uuidV7Schema,
    agentInstanceId: uuidV7Schema.optional(),
    audience: z.enum(automationAudiences),
    impactAcknowledged: z.literal(true),
    lifetimeSeconds: z
      .number()
      .int()
      .min(1)
      .max(30 * 24 * 60 * 60),
    maxMessagesPerMinute: z.number().int().min(1).max(60),
    maxTotalMessages: z.number().int().min(1).max(10_000).optional(),
    messageKinds: z.array(z.enum(automationMessageKinds)).min(1).max(2),
    requiresRiskScan: z.boolean(),
    roomCatalogId: uuidV7Schema,
  })
  .strict();

export type AutomationGrant = z.output<typeof automationGrantSchema>;
export type CreateAutomationGrantInput = z.input<typeof createAutomationGrantInputSchema>;

export type AutomationGrantFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

export type AutomationGrantGateway = {
  create(
    grantId: string,
    input: CreateAutomationGrantInput,
  ): Promise<Result<AutomationGrant, AutomationGrantFailure>>;
  list(): Promise<Result<readonly AutomationGrant[], AutomationGrantFailure>>;
  revoke(grantId: string): Promise<Result<AutomationGrant, AutomationGrantFailure>>;
};

export function orderAutomationGrants(
  grants: readonly AutomationGrant[],
): readonly AutomationGrant[] {
  return Object.freeze([...grants].toSorted(compareAutomationGrants));
}

export function isAutomationGrantActive(grant: AutomationGrant, nowUnixMs: number): boolean {
  return grant.status === 'active' && grant.expiresAtUnixMs > nowUnixMs;
}

function compareAutomationGrants(left: AutomationGrant, right: AutomationGrant): number {
  const stateDifference = Number(left.status !== 'active') - Number(right.status !== 'active');
  if (stateDifference !== 0) {
    return stateDifference;
  }
  const expiryDifference = right.expiresAtUnixMs - left.expiresAtUnixMs;
  return expiryDifference === 0 ? right.grantId.localeCompare(left.grantId) : expiryDifference;
}
