import { z } from 'zod';

import type { Result } from '@/shared/result';

export const moderationReasons = [
  'spam',
  'harassment',
  'impersonation',
  'malicious_content',
  'privacy_violation',
  'unsafe_automation',
  'other',
] as const;
export const moderationActionKinds = ['hide', 'mute', 'kick', 'ban'] as const;
export const moderationTargetKinds = [
  'principal',
  'agent',
  'room',
  'event',
  'federation_peer',
] as const;

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const timestampSchema = z.number().int().nonnegative();
const referenceSchema = z.string().min(1).max(1_024);

export const moderationEvidenceSchema = z
  .object({
    endToEndEncrypted: z.boolean(),
    matrixEventId: z.string().min(1).max(1_024).nullable(),
    reporterSubmittedExcerpt: z.string().min(1).max(4_096).nullable(),
    roomCatalogId: uuidV7Schema.nullable(),
  })
  .strict()
  .superRefine((evidence, context) => {
    if (evidence.reporterSubmittedExcerpt !== null && evidence.matrixEventId === null) {
      context.addIssue({ code: 'custom', message: '显式摘录必须绑定 Matrix 事件。' });
    }
  });

export const moderationCaseSchema = z
  .object({
    caseId: uuidV7Schema,
    createdAtUnixMs: timestampSchema,
    description: z.string().max(4_096),
    evidence: moderationEvidenceSchema,
    reason: z.enum(moderationReasons),
    resolvedAtUnixMs: timestampSchema.nullable(),
    state: z.enum(['open', 'in_review', 'resolved', 'dismissed']),
    targetKind: z.enum(moderationTargetKinds),
    targetReference: referenceSchema,
  })
  .strict();

export const moderationActionSchema = z
  .object({
    actionId: uuidV7Schema,
    actorPrincipalId: uuidV7Schema,
    caseId: uuidV7Schema.nullable(),
    expiresAtUnixMs: timestampSchema.nullable(),
    failureCode: z.string().min(1).max(128).nullable(),
    kind: z.enum(moderationActionKinds),
    reason: z.enum(moderationReasons),
    reversedAtUnixMs: timestampSchema.nullable(),
    roomCatalogId: uuidV7Schema,
    startsAtUnixMs: timestampSchema,
    status: z.enum(['pending', 'applied', 'failed', 'reversed']),
    targetKind: z.enum(moderationTargetKinds),
    targetReference: referenceSchema,
  })
  .strict();

export const moderationAuditEventSchema = z
  .object({
    action: z.string().min(1).max(128),
    actorPrincipalId: uuidV7Schema,
    correlationId: z.uuid(),
    eventId: uuidV7Schema,
    occurredAtUnixMs: timestampSchema,
    outcome: z.enum(['allowed', 'denied', 'failed']),
    reason: z.enum(moderationReasons).nullable(),
    roomCatalogId: uuidV7Schema.nullable(),
    targetKind: z.enum(moderationTargetKinds),
    targetReference: referenceSchema,
  })
  .strict();

export const moderationCaseListSchema = z
  .object({ cases: z.array(moderationCaseSchema).max(1_000).readonly() })
  .strict();
export const moderationActionListSchema = z
  .object({ actions: z.array(moderationActionSchema).max(1_000).readonly() })
  .strict();
export const moderationAuditListSchema = z
  .object({ events: z.array(moderationAuditEventSchema).max(500).readonly() })
  .strict();

export const submitModerationReportInputSchema = z
  .object({
    description: z.string().max(4_096),
    evidence: z
      .object({
        endToEndEncrypted: z.boolean(),
        matrixEventId: z.string().min(1).max(1_024).optional(),
        reporterSubmittedExcerpt: z.string().min(1).max(4_096).optional(),
        roomCatalogId: uuidV7Schema.optional(),
      })
      .strict()
      .superRefine((evidence, context) => {
        if (
          evidence.reporterSubmittedExcerpt !== undefined &&
          evidence.matrixEventId === undefined
        ) {
          context.addIssue({ code: 'custom', message: '显式摘录必须绑定 Matrix 事件。' });
        }
      }),
    reason: z.enum(moderationReasons),
    targetKind: z.enum(moderationTargetKinds),
    targetReference: referenceSchema,
  })
  .strict();

export const applyModerationActionInputSchema = z
  .object({
    caseId: uuidV7Schema.optional(),
    expiresAtUnixMs: timestampSchema.optional(),
    impactAcknowledged: z.literal(true),
    kind: z.enum(moderationActionKinds),
    reason: z.enum(moderationReasons),
    targetKind: z.enum(moderationTargetKinds),
    targetReference: referenceSchema,
  })
  .strict()
  .superRefine((action, context) => {
    const expectedTarget = action.kind === 'hide' ? 'event' : 'principal';
    if (action.targetKind !== expectedTarget) {
      context.addIssue({ code: 'custom', message: '治理动作与目标类别不匹配。' });
    }
  });

export type ModerationReason = (typeof moderationReasons)[number];
export type ModerationActionKind = (typeof moderationActionKinds)[number];
export type ModerationCase = z.output<typeof moderationCaseSchema>;
export type ModerationAction = z.output<typeof moderationActionSchema>;
export type ModerationAuditEvent = z.output<typeof moderationAuditEventSchema>;
export type SubmitModerationReportInput = z.input<typeof submitModerationReportInputSchema>;
export type ApplyModerationActionInput = z.input<typeof applyModerationActionInputSchema>;

export type ModerationFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryAfterSeconds?: number;
  readonly retryable: boolean;
};

export type ModerationGateway = {
  applyAction(
    actionId: string,
    roomCatalogId: string,
    input: ApplyModerationActionInput,
  ): Promise<Result<ModerationAction, ModerationFailure>>;
  listActions(
    roomCatalogId: string,
  ): Promise<Result<readonly ModerationAction[], ModerationFailure>>;
  listAudit(
    roomCatalogId: string,
  ): Promise<Result<readonly ModerationAuditEvent[], ModerationFailure>>;
  listCases(): Promise<Result<readonly ModerationCase[], ModerationFailure>>;
  listRoomCases(
    roomCatalogId: string,
  ): Promise<Result<readonly ModerationCase[], ModerationFailure>>;
  report(
    caseId: string,
    input: SubmitModerationReportInput,
  ): Promise<Result<ModerationCase, ModerationFailure>>;
  reverseAction(actionId: string): Promise<Result<ModerationAction, ModerationFailure>>;
};
