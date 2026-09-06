import { z } from 'zod';
import type { Result } from '@/shared/result';

export const agentRecoverySessionSchema = z
  .object({
    sessionId: z.uuid(),
    displayName: z.string().min(1).max(128),
    state: z.enum(['starting', 'ready', 'failed', 'closed']),
  })
  .strict();
export const agentRecoveryStateSchema = z
  .object({
    userId: z.string().min(4).max(512),
    deviceId: z.string().min(1).max(255),
    identity: z.enum(['missing', 'ready', 'recovery_required']),
    recoveryAvailable: z.boolean(),
    backupEnabled: z.boolean(),
  })
  .strict();
export const agentRecoveryResultSchema = z
  .object({
    state: agentRecoveryStateSchema,
    recoveryKey: z.string().min(1).max(512).nullable(),
  })
  .strict();
export const agentRecoveryRequestSchema = z.discriminatedUnion('action', [
  z.object({ action: z.literal('inspect') }).strict(),
  z.object({ action: z.literal('enable'), passphrase: recoveryCredential(12) }).strict(),
  z.object({ action: z.literal('restore'), credential: recoveryCredential(1) }).strict(),
]);

function recoveryCredential(minimum: number) {
  return z
    .string()
    .refine(
      (value) =>
        Array.from(value.trim()).length >= minimum &&
        new TextEncoder().encode(value).length <= 1024 &&
        !/\p{Cc}/u.test(value),
    );
}
export type AgentRecoveryRequest = z.infer<typeof agentRecoveryRequestSchema>;
export type AgentRecoveryResult = z.infer<typeof agentRecoveryResultSchema>;
export type AgentRecoverySession = z.infer<typeof agentRecoverySessionSchema>;
export type AgentRecoveryFailure = { readonly code: string; readonly retryable: boolean };
export type AgentRecoveryGateway = {
  sessions(): Promise<Result<readonly AgentRecoverySession[], AgentRecoveryFailure>>;
  execute(
    sessionId: string,
    request: AgentRecoveryRequest,
  ): Promise<Result<AgentRecoveryResult, AgentRecoveryFailure>>;
};
