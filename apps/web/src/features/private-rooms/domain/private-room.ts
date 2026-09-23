import { z } from 'zod';

import type { Result } from '@/shared/result';

export const privateRoomCapabilities = ['view', 'speak', 'invite', 'manage', 'automate'] as const;

export type PrivateRoomCapability = (typeof privateRoomCapabilities)[number];

export const privateRoomPrincipalIdSchema = z.uuid();

/// 邀请只接受 Agent Room 主体 ID。产品不提供按昵称或邮箱检索账号，因此先在本地判断格式，
/// 把明显错误的输入挡在请求之前，并让界面给出可操作的提示。
export function isPrivateRoomPrincipalId(value: string): boolean {
  return privateRoomPrincipalIdSchema.safeParse(value.trim()).success;
}

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
    principalId: privateRoomPrincipalIdSchema,
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

const unixMillisSchema = z.number().int().nonnegative();

/// 凭口令进来的 Agent：它是房间的 Agent 成员，主人并不因此成为房间成员。
const agentMemberSchema = z
  .object({
    agentId: z.uuid(),
    displayName: z.string().min(1),
    joinedAtUnixMs: unixMillisSchema,
    ownerDisplayName: z.string().nullable(),
    status: z.enum(['joined', 'removed']),
    statusChangedAtUnixMs: unixMillisSchema,
  })
  .strict();

/// 口令是否开着（只有创建时间，不含口令本身）与凭口令进来的 Agent。
export const privateRoomAgentAccessSchema = z
  .object({
    agents: z.array(agentMemberSchema),
    joinCode: z.object({ createdAtUnixMs: unixMillisSchema }).strict().nullable(),
  })
  .strict();

/// 口令只在生成的那次响应里出现：12 位 Crockford Base32，分三组。
export const generatedJoinCodeSchema = z
  .object({
    code: z.string().regex(/^[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}$/u),
    createdAtUnixMs: unixMillisSchema,
  })
  .strict();

export type PrivateRoom = z.infer<typeof privateRoomSchema>;
export type PrivateRoomAgentAccess = z.infer<typeof privateRoomAgentAccessSchema>;
export type PrivateRoomAgentMember = z.infer<typeof agentMemberSchema>;
export type GeneratedJoinCode = z.infer<typeof generatedJoinCodeSchema>;
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
  /** Join code status and the agents that came in with it; needs the manage permission. */
  agentAccess(catalogId: string): Promise<Result<PrivateRoomAgentAccess, PrivateRoomFailure>>;
  archive(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  ban(catalogId: string, principalId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  create(
    catalogId: string,
    input: CreatePrivateRoomInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  decline(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  /** Turns the join code off; agents that already joined stay. */
  disableJoinCode(catalogId: string): Promise<Result<void, PrivateRoomFailure>>;
  /** Creates or replaces the join code. The code is returned only this once. */
  generateJoinCode(catalogId: string): Promise<Result<GeneratedJoinCode, PrivateRoomFailure>>;
  inspect(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  invite(
    catalogId: string,
    invitation: PrivateRoomInvitationInput,
  ): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  leave(catalogId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  list(): Promise<Result<readonly PrivateRoom[], PrivateRoomFailure>>;
  remove(catalogId: string, principalId: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
  /** Removes an agent that joined with the code; it needs a newer code to come back. */
  removeCodeAgent(catalogId: string, agentId: string): Promise<Result<void, PrivateRoomFailure>>;
  /** Only the owner may rename; the Matrix room name follows. */
  rename(catalogId: string, name: string): Promise<Result<PrivateRoom, PrivateRoomFailure>>;
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
