import { z } from 'zod';
import type { Result } from '@/shared/result';
const pending = z
  .object({ eventId: z.string().max(2048), messageId: z.uuidv7(), submissionId: z.uuidv7() })
  .strict();
export const receptionRecordSchema = z
  .object({
    agentId: z.uuidv7(),
    catalogId: z.uuidv7(),
    roomId: z.string().min(1).max(512),
    sessionKey: z.uuidv7(),
    displayName: z.string().min(1).max(128),
    instanceId: z.uuidv7(),
    deviceId: z.uuidv7(),
    deviceLabel: z.string().max(128),
    runId: z.uuidv7(),
    status: z.enum(['active', 'draining', 'idle']),
    nextDeviceId: z.uuidv7().nullable(),
    lastSeenUnixMs: z.number().int().nonnegative(),
    revision: z.number().int().nonnegative(),
    progress: z
      .object({
        afterEventId: z.string().max(2048).nullable(),
        pending: pending.nullable(),
        retry: pending.nullable(),
      })
      .strict(),
  })
  .strict();
export type ReceptionRecord = z.infer<typeof receptionRecordSchema>;
export type ReceptionTransfer = {
  readonly agentId: string;
  readonly catalogId: string;
  readonly nextDeviceId: string | null;
};
export type ReceptionOwnershipFailure = { readonly code: string; readonly retryable: boolean };
export type ReceptionOwnershipGateway = {
  readonly list: () => Promise<
    Result<
      { readonly receptions: readonly ReceptionRecord[]; readonly limited: boolean },
      ReceptionOwnershipFailure
    >
  >;
  readonly transfer: (
    request: ReceptionTransfer,
  ) => Promise<Result<ReceptionRecord, ReceptionOwnershipFailure>>;
};
export function receptionAvailability(record: ReceptionRecord, now: number) {
  if (record.status === 'idle') return 'idle';
  if (record.status === 'draining') return 'draining';
  return now - record.lastSeenUnixMs <= 20000 ? 'active' : 'unreachable';
}
