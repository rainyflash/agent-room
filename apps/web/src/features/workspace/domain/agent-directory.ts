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

/**
 * Agent 在的一个房间（`specs/agent-identity/design.md`“控制面”）：按服务器上占着的位置算，同一个房间
 * 只有一条。公共大厅按目录算，不分分片。
 */
export const agentRoomSchema = z
  .object({
    archived: z.boolean(),
    catalogId: uuidV7Schema,
    kind: z.enum(['public_lobby', 'private_room']),
    name: z.string().min(1),
  })
  .strict();

export const agentRoomListSchema = z
  .object({ rooms: z.array(agentRoomSchema).max(2_000) })
  .strict();

/** 带进房间：进去了几个连接。是 0 时它在你的电脑上没有能用的连接。 */
export const broughtAgentSchema = z.object({ entered: z.number().int().nonnegative() }).strict();

/** 离开：原本就不在这个房间时 `left` 为 `false`。 */
export const leftAgentRoomSchema = z.object({ left: z.boolean() }).strict();

export type AgentRoom = z.output<typeof agentRoomSchema>;

/**
 * 一个还在用的连接，它的 Matrix 设备上次用是什么时候（Synapse 记的，等消息、发言都算）。`null` 是设备
 * 已经不在了或者从没用过。问不到的连接不在列表里。
 */
export const agentInstanceActivitySchema = z
  .object({
    agentInstanceId: uuidV7Schema,
    matrixLastSeenAtUnixMs: z.number().int().nonnegative().nullable(),
  })
  .strict();

export const agentInstanceActivityListSchema = z
  .object({ instances: z.array(agentInstanceActivitySchema).max(2_000) })
  .strict();

/** 按连接 ID 记的设备上次用的时间。 */
export type AgentInstanceActivity = ReadonlyMap<string, number | null>;

export type AgentDirectoryFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryable: boolean;
  /** 房间正在准备（`agent_room.busy`）时服务器说的过多久再试。 */
  readonly retryAfterSeconds?: number;
};

export type AgentDirectoryGateway = {
  listOwnedAgents(): Promise<Result<readonly OwnedAgent[], AgentDirectoryFailure>>;
  /** Retire an agent this account solely owns; its instances must already be revoked. */
  deleteAgent(agentId: string): Promise<Result<void, AgentDirectoryFailure>>;
  /** 它在哪些房间。 */
  listAgentRooms(agentId: string): Promise<Result<readonly AgentRoom[], AgentDirectoryFailure>>;
  /** 每个还在用的连接的设备上次用的时间（`GET /agent-instances/matrix-activity`）。 */
  listInstanceActivity(): Promise<Result<AgentInstanceActivity, AgentDirectoryFailure>>;
  /** 把它在你电脑上的连接带进一个房间，回答进去了几个连接。 */
  bringAgentIntoRoom(
    agentId: string,
    catalogId: string,
  ): Promise<Result<number, AgentDirectoryFailure>>;
  /** 让它离开一个房间，回答它原本在不在。 */
  leaveAgentRoom(
    agentId: string,
    catalogId: string,
  ): Promise<Result<boolean, AgentDirectoryFailure>>;
};
