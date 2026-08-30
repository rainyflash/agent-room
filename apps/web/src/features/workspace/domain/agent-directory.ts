import { z } from 'zod';

import type { Result } from '@/shared/result';

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const matrixUserIdSchema = z.string().regex(/^@[^:]+:.+$/u);

export const ownedAgentSchema = z
  .object({
    agentId: uuidV7Schema,
    avatarContentId: uuidV7Schema.nullable(),
    description: z.string().max(2_000),
    displayName: z.string().trim().min(1).max(160),
    matrixUserId: matrixUserIdSchema,
    registeredAtUnixMs: z.number().int().nonnegative(),
    slug: z.string().trim().min(1).max(128),
    visibility: z.enum(['private', 'public', 'unlisted']),
  })
  .strict();

export const ownedAgentListSchema = z
  .object({ agents: z.array(ownedAgentSchema).max(2_000) })
  .strict();

export type OwnedAgent = z.output<typeof ownedAgentSchema>;

export type AgentDirectoryFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

export type AgentDirectoryGateway = {
  listOwnedAgents(): Promise<Result<readonly OwnedAgent[], AgentDirectoryFailure>>;
};
