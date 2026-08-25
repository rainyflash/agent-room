import { z } from 'zod';

import type { Result } from '@/shared/result';

export const privateRoomCapabilities = ['view', 'speak', 'invite', 'manage', 'automate'] as const;

export type PrivateRoomCapability = (typeof privateRoomCapabilities)[number];

const permissionsSchema = z
  .object({
    capabilities: z.array(z.enum(privateRoomCapabilities)).max(privateRoomCapabilities.length),
  })
  .strict()
  .transform(({ capabilities }) => ({
    capabilities: Object.freeze(Array.from(new Set(capabilities))),
  }));

const memberSchema = z
  .object({
    permissions: permissionsSchema,
    principalId: z.uuid(),
    status: z.enum(['invited', 'joined', 'declined', 'removed', 'banned']),
  })
  .strict();

export const privateRoomSchema = z
  .object({
    catalogId: z.uuid(),
    description: z.string(),
    matrixRoomId: z.string().regex(/^![^:]+:.+$/u),
    members: z.array(memberSchema),
    name: z.string().trim().min(1),
    ownerPrincipalId: z.uuid(),
    retentionDays: z.number().int().positive().nullable(),
    roomInstanceId: z.uuid(),
    status: z.enum(['active', 'archived']),
    version: z.number().int().nonnegative(),
  })
  .strict();

export const privateRoomListSchema = z.object({ rooms: z.array(privateRoomSchema) }).strict();

export type PrivateRoom = z.infer<typeof privateRoomSchema>;
export type PrivateRoomMember = z.infer<typeof memberSchema>;
export type PrivateRoomPermissions = z.infer<typeof permissionsSchema>;

export type PrivateRoomFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryable: boolean;
};

export type PrivateRoomInvitationInput = {
  readonly permissions: PrivateRoomPermissions;
  readonly principalId: string;
};

export type CreatePrivateRoomInput = {
  readonly description: string;
  readonly invitations: readonly PrivateRoomInvitationInput[];
  readonly name: string;
  readonly retentionDays?: number;
};

export type TransferPrivateRoomOwnershipInput = {
  readonly formerOwnerPermissions: PrivateRoomPermissions;
  readonly targetPrincipalId: string;
};

export type PrivateRoomGateway = {
  accept(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  archive(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  ban(catalogId: string, principalId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  create(
    catalogId: string,
    input: CreatePrivateRoomInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  decline(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  inspect(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  invite(
    catalogId: string,
    invitation: PrivateRoomInvitationInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  leave(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  list(): Promise<Result<readonly PrivateRoom[], PrivateRoomFailure>>;
  remove(catalogId: string, principalId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  transferOwnership(
    catalogId: string,
    input: TransferPrivateRoomOwnershipInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  updatePermissions(
    catalogId: string,
    principalId: string,
    permissions: PrivateRoomPermissions,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
};

export type PrivateRoomMatrixGateway = {
  join(roomId: string): Promise<Result<void, PrivateRoomFailure>>;
  leave(roomId: string): Promise<Result<void, PrivateRoomFailure>>;
};

export function permissions(...capabilities: PrivateRoomCapability[]): PrivateRoomPermissions {
  return { capabilities: Object.freeze(Array.from(new Set(capabilities))) };
}

export function allows(
  member: PrivateRoomMember | undefined,
  capability: PrivateRoomCapability,
): boolean {
  return member?.status === 'joined' && member.permissions.capabilities.includes(capability);
}

export function memberFor(room: PrivateRoom, principalId: string): PrivateRoomMember | undefined {
  return room.members.find((member) => member.principalId === principalId);
}
