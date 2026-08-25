import { z } from 'zod';

import type { Result } from '@/shared/result';

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const timestampSchema = z.number().int().nonnegative();

export const productDeviceSchema = z
  .object({
    createdAtUnixMs: timestampSchema,
    deviceId: uuidV7Schema,
    label: z.string().min(1).max(128),
    lastSeenAtUnixMs: timestampSchema.nullable(),
    matrixDeviceId: z.string().min(1).max(255).nullable(),
    platform: z.enum(['windows', 'macos', 'linux', 'web']),
    revokedAtUnixMs: timestampSchema.nullable(),
    trustState: z.enum(['pending', 'verified', 'revoked']),
  })
  .strict();

export const productDeviceListSchema = z
  .object({ devices: z.array(productDeviceSchema).max(500) })
  .strict();

const agentInstanceDeviceSchema = z
  .object({
    deviceId: uuidV7Schema,
    label: z.string().min(1).max(128),
    platform: z.enum(['windows', 'macos', 'linux', 'web']),
    trustState: z.enum(['pending', 'verified', 'revoked']),
  })
  .strict();

export const agentInstanceSchema = z
  .object({
    adapterType: z.string().min(1).max(128),
    agentAvatarContentId: uuidV7Schema.nullable(),
    agentDisplayName: z.string().min(1).max(80),
    agentId: uuidV7Schema,
    agentInstanceId: uuidV7Schema,
    capabilityVersion: z.string().min(1).max(128),
    createdAtUnixMs: timestampSchema,
    device: agentInstanceDeviceSchema,
    lastSeenAtUnixMs: timestampSchema.nullable(),
    matrixDeviceId: z.string().min(1).max(255),
    matrixDeviceRevokedAtUnixMs: timestampSchema.nullable(),
    revokedAtUnixMs: timestampSchema.nullable(),
    status: z.enum(['connecting', 'online', 'degraded', 'offline', 'revoked']),
  })
  .strict();

export const agentInstanceListSchema = z
  .object({ instances: z.array(agentInstanceSchema).max(2_000) })
  .strict();

export const pendingProductDeviceRevocationSchema = z
  .object({
    localRevocation: z.literal('complete'),
    matrixCleanup: z.literal('pending'),
    pendingAgentInstanceCount: z.number().int().positive(),
  })
  .strict();

export const agentInstanceRevocationSchema = z
  .object({
    instance: agentInstanceSchema,
    matrixCleanup: z.enum(['complete', 'pending']),
    matrixCleanupPendingReason: z
      .enum([
        'dependencyUnavailable',
        'rejected',
        'unsupported',
        'invalidStoredIdentity',
        'statePersistenceUnavailable',
      ])
      .nullable(),
  })
  .strict()
  .superRefine((revocation, context) => {
    const pending = revocation.matrixCleanup === 'pending';
    if (pending !== (revocation.matrixCleanupPendingReason !== null)) {
      context.addIssue({ code: 'custom', message: 'Matrix 清理状态与原因互相矛盾。' });
    }
  });

export type ProductDevice = z.output<typeof productDeviceSchema>;
export type AgentInstance = z.output<typeof agentInstanceSchema>;
export type AgentInstanceRevocation = z.output<typeof agentInstanceRevocationSchema>;

export type ProductDeviceRevocation =
  | { readonly matrixCleanup: 'complete'; readonly pendingAgentInstanceCount: 0 }
  | { readonly matrixCleanup: 'pending'; readonly pendingAgentInstanceCount: number };

export type AccessManagementFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

export type AccessManagementGateway = {
  listAgentInstances(): Promise<Result<readonly AgentInstance[], AccessManagementFailure>>;
  listProductDevices(): Promise<Result<readonly ProductDevice[], AccessManagementFailure>>;
  revokeAgentInstance(
    instanceId: string,
  ): Promise<Result<AgentInstanceRevocation, AccessManagementFailure>>;
  revokeProductDevice(
    deviceId: string,
  ): Promise<Result<ProductDeviceRevocation, AccessManagementFailure>>;
};
