import { z } from 'zod';
import type { AutomationGrant } from '@/features/automation/domain/automation-grant';

const failure = z
  .object({
    code: z.string(),
    category: z.string(),
    retryable: z.boolean(),
    details: z.record(z.string(), z.string()),
  })
  .strict();
const delivery = z
  .object({
    eventId: z.string(),
    messageId: z.uuid(),
    submissionId: z.uuid(),
    stage: z.enum(['received', 'running', 'verifying', 'replied', 'needs_review', 'skipped']),
    replyEventId: z.string().nullable(),
    failure: failure.nullable(),
  })
  .strict();
const start = z.discriminatedUnion('mode', [
  z.object({ mode: z.literal('now') }).strict(),
  z.object({ mode: z.literal('beginning') }).strict(),
  z.object({ mode: z.literal('after'), eventId: z.string() }).strict(),
]);
const checkpoint = z.discriminatedUnion('state', [
  z.object({ state: z.literal('ready'), afterEventId: z.string().nullable() }).strict(),
  z
    .object({
      state: z.literal('pending'),
      afterEventId: z.string().nullable(),
      eventId: z.string(),
    })
    .strict(),
]);
export const receiverViewSchema = z
  .object({
    running: z.boolean(),
    failure: failure.nullable(),
    progress: z
      .discriminatedUnion('type', [
        z
          .object({
            type: z.literal('ready'),
            agentId: z.uuid(),
            hostTaskId: z.uuid(),
            roomId: z.string(),
          })
          .strict(),
        z.object({ type: z.literal('delivery'), record: delivery }).strict(),
        z
          .object({
            type: z.literal('reconnecting'),
            error: failure,
            retryInSeconds: z.number().nonnegative(),
          })
          .strict(),
        z.object({ type: z.literal('stopped') }).strict(),
      ])
      .nullable(),
    state: z
      .object({
        binding: z
          .object({
            session: z.object({ sessionKey: z.uuid(), displayName: z.string() }).strict(),
            policy: z.object({ roomId: z.string(), allowedPrincipalId: z.uuid() }).strict(),
            automationGrantId: z.uuid(),
            host: z
              .object({
                taskId: z.uuid(),
                executable: z.string(),
                mcpExecutable: z.string(),
                workspace: z.string(),
              })
              .strict(),
            start,
          })
          .strict(),
        bridgeService: z.string(),
        agentId: z.uuid().nullable(),
        roomCatalogId: z.uuid().nullable(),
        instanceId: z.uuid().nullable(),
        checkpoint,
        enabled: z.boolean(),
        lastDelivery: delivery.nullable(),
      })
      .strict(),
  })
  .strict();
export type ReceiverView = z.infer<typeof receiverViewSchema>;
export type ConfigureReceiver = {
  readonly sessionId: string;
  readonly principalId: string;
  readonly automationGrantId: string;
  readonly executable: string | null;
};
export type ReceiverAction =
  | { readonly action: 'start' | 'verify' | 'pause' | 'remove' }
  | { readonly action: 'resolve'; readonly event: string; readonly resolution: 'retry' | 'skip' }
  | {
      readonly action: 'update';
      readonly automationGrantId: string;
      readonly executable: string | null;
      readonly workspace: string | null;
    };

export function receptionGrants(
  grants: readonly AutomationGrant[],
  agentId: string | null,
  catalogId: string | null,
  instanceId: string | null,
  now: number,
): readonly AutomationGrant[] {
  return grants.filter(
    (grant) =>
      grant.agentId === agentId &&
      grant.roomCatalogId === catalogId &&
      (grant.agentInstanceId === null || grant.agentInstanceId === instanceId) &&
      grant.status === 'active' &&
      grant.startsAtUnixMs <= now &&
      grant.expiresAtUnixMs > now &&
      grant.messageKinds.includes('reply') &&
      (grant.maxTotalMessages === null || grant.totalMessages < grant.maxTotalMessages),
  );
}
