import { z } from 'zod';
import type { Result } from '@/shared/result';

export const agentRosterPolicySchema = z
  .object({
    schemaVersion: z.literal(1),
    archiveAfterDays: z.union([z.literal(1), z.literal(7), z.literal(30)]),
  })
  .strict();
export type AgentRosterPolicy = z.output<typeof agentRosterPolicySchema>;
export type AgentRosterPolicyFailure = {
  readonly code: 'unavailable' | 'forbidden' | 'invalid_response';
};
export type AgentRosterPolicyGateway = {
  update(
    catalogId: string,
    policy: AgentRosterPolicy,
  ): Promise<Result<AgentRosterPolicy, AgentRosterPolicyFailure>>;
};
