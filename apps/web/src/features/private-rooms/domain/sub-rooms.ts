import { z } from 'zod';

import type { Result } from '@/shared/result';
import type { PrivateRoom, PrivateRoomFailure } from './private-room';
import type { JoinRequestDecision, PendingJoinRequest } from './public-room-join';

/**
 * 子房间（`specs/redesign/sub-rooms.md`）：一间私人房间下面一件事一间的小房间。这里的回答都不加
 * `.strict()`：子房间还在一步步加东西，已经装着的网页要读得了以后多出来的项。
 */
const unixMillisSchema = z.number().int().nonnegative();
const countSchema = z.number().int().nonnegative();

/** 谁能进：`open` 没有门，`request` 要批准，`invite_only` 只能请进来；和公开房间的加入方式是同一套词。 */
export const subRoomAccessValues = ['open', 'request', 'invite_only'] as const;
export const subRoomAccessSchema = z.enum(subRoomAccessValues);

/** 分类：整间房共用，按先后（`position` 从 0 起、连着）。 */
export const subRoomCategorySchema = z.object({
  categoryId: z.uuid(),
  name: z.string().min(1),
  position: countSchema,
});

export const subRoomSchema = z.object({
  catalogId: z.uuid(),
  name: z.string().min(1),
  description: z.string(),
  access: subRoomAccessSchema,
  /** 不在名单里显示：只有进去过的和管父房间的看得到。 */
  hidden: z.boolean(),
  /** 在哪个分类里；“未分类”的没有。 */
  categoryId: z.uuid().optional(),
  createdByPrincipalId: z.uuid(),
  createdAtUnixMs: unixMillisSchema,
  /** 这一间的子房间管理员；房间的房主和管理员本来就管每一间，不在这里。 */
  managerPrincipalIds: z.array(z.uuid()),
  /** 你在不在里面。 */
  joined: z.boolean(),
  /** 你是不是被这一间封禁了。 */
  banned: z.boolean(),
  /** 人数、Agent 数、最近有人说话的时间：有门、你又不在里面时没有。 */
  memberCount: countSchema.optional(),
  agentCount: countSchema.optional(),
  lastMessageAtUnixMs: unixMillisSchema.optional(),
  /** 你在等批准时，什么时候申请的。 */
  requestedAtUnixMs: unixMillisSchema.optional(),
  /** 在等的申请有几个：只给管得了这一间的人。 */
  pendingRequestCount: countSchema.optional(),
  /** 下面四项只在“已归档”的名单里有。自动归档的、跟着父房间归档的没有“谁”。 */
  archivedAtUnixMs: unixMillisSchema.optional(),
  archivedByPrincipalId: z.uuid().optional(),
  archivedReason: z.enum(['manual', 'inactive', 'parent']).optional(),
  canRestore: z.boolean().optional(),
});

export const subRoomListSchema = z.object({
  parent: z.object({ catalogId: z.uuid(), name: z.string().min(1) }),
  /** 你此刻能不能在这间房下面建子房间。 */
  canCreate: z.boolean(),
  categories: z.array(subRoomCategorySchema),
  /** 没归档的按建的先后；“已归档”的那份最近归档的在前。 */
  subRooms: z.array(subRoomSchema),
});

export const subRoomCategoriesSchema = z.object({ categories: z.array(subRoomCategorySchema) });

/** 申请以后：`pending` 在等批准；`joined` 不用等（门刚拆掉、本来就在里面、你管父房间），已经记成已加入。 */
export const subRoomJoinStateSchema = z.object({
  catalogId: z.uuid(),
  membership: z.enum(['pending', 'joined']),
  requestedAtUnixMs: unixMillisSchema.optional(),
});

/** 房间设置里的“子房间”一节：谁能建、Agent 能不能建、30 天没人说话的自动归档；只有房主改得动。 */
export const subRoomPolicySchema = z.object({
  creators: z.enum(['members', 'admins']),
  agentsMayCreate: z.boolean(),
  autoArchive: z.boolean(),
  canChange: z.boolean(),
});

/** 你管得了这一间：服务器只给管得了的人写在等的申请有几个（这一间的管理员、父房间的房主和管理员）。 */
export const managesSubRoom = (room: SubRoom): boolean => room.pendingRequestCount !== undefined;

export type SubRoomAccess = z.infer<typeof subRoomAccessSchema>;
export type SubRoomCategory = z.infer<typeof subRoomCategorySchema>;
export type SubRoom = z.infer<typeof subRoomSchema>;
export type SubRoomList = z.infer<typeof subRoomListSchema>;
export type SubRoomJoinState = z.infer<typeof subRoomJoinStateSchema>;
export type SubRoomPolicy = z.infer<typeof subRoomPolicySchema>;
export type SubRoomPolicySettings = Omit<SubRoomPolicy, 'canChange'>;

export type CreateSubRoomInput = {
  /** 新子房间的房间号，由网页先定好，也是这次建房的幂等键：同一个号再建一次拿回同一间。 */
  readonly catalogId: string;
  readonly name: string;
  readonly description?: string;
  /** 不带是 `open`。 */
  readonly access?: SubRoomAccess;
  readonly hidden?: boolean;
  readonly categoryId?: string;
};

/** 没带的那一项不动；`categoryId: null` 是挪回“未分类”。 */
export type ReviseSubRoomInput = {
  readonly name?: string;
  readonly description?: string;
  readonly access?: SubRoomAccess;
  readonly hidden?: boolean;
  readonly categoryId?: string | null;
};

/** 建分类：分类号由网页先定好，也是这次建的幂等键（`Idempotency-Key`），同一个号再建一次不多建。 */
export type CreateSubRoomCategoryInput = {
  readonly categoryId: string;
  readonly name: string;
};

export type ReviseSubRoomCategoryInput = {
  readonly name?: string;
  readonly position?: number;
};

/** 子房间的地址：父房间的号加这一间的号。 */
export type SubRoomRef = {
  readonly parentId: string;
  readonly subRoomId: string;
};

type SubRoomResult<T> = Promise<Result<T, PrivateRoomFailure>>;

export type SubRoomGateway = {
  /**
   * The sub-rooms you can see under a room. `roomId` may be the parent or any of its sub-rooms, so a
   * sub-room page learns its parent this way. `archived` asks for the archived ones instead.
   */
  subRooms(roomId: string, options?: { readonly archived?: boolean }): SubRoomResult<SubRoomList>;
  create(parentId: string, input: CreateSubRoomInput): SubRoomResult<PrivateRoom>;
  revise(room: SubRoomRef, input: ReviseSubRoomInput): SubRoomResult<PrivateRoom>;
  archive(room: SubRoomRef): SubRoomResult<void>;
  restore(room: SubRoomRef): SubRoomResult<void>;
  /** Joins one without a door; the page then joins on Matrix as for an invited member. */
  join(room: SubRoomRef): SubRoomResult<PrivateRoom>;
  /** Leaves; the control plane leaves on Matrix for you. */
  leave(room: SubRoomRef): SubRoomResult<void>;
  /** Asks to join one that needs approval, with a message of up to 200 characters. */
  requestToJoin(room: SubRoomRef, message: string): SubRoomResult<SubRoomJoinState>;
  withdrawJoinRequest(room: SubRoomRef): SubRoomResult<void>;
  /** Requests waiting for an answer, oldest first; people who manage this sub-room can ask. */
  joinRequests(room: SubRoomRef): SubRoomResult<readonly PendingJoinRequest[]>;
  answerJoinRequest(
    room: SubRoomRef,
    principalId: string,
    decision: JoinRequestDecision,
  ): SubRoomResult<void>;
  /** Brings someone from the parent room straight in. */
  admit(room: SubRoomRef, principalId: string): SubRoomResult<void>;
  remove(room: SubRoomRef, principalId: string): SubRoomResult<void>;
  ban(room: SubRoomRef, principalId: string): SubRoomResult<void>;
  unban(room: SubRoomRef, principalId: string): SubRoomResult<void>;
  grantManager(room: SubRoomRef, principalId: string): SubRoomResult<void>;
  revokeManager(room: SubRoomRef, principalId: string): SubRoomResult<void>;
  /** Category writes answer the room's categories after the change, in order. */
  createCategory(
    parentId: string,
    input: CreateSubRoomCategoryInput,
  ): SubRoomResult<readonly SubRoomCategory[]>;
  reviseCategory(
    parentId: string,
    categoryId: string,
    input: ReviseSubRoomCategoryInput,
  ): SubRoomResult<readonly SubRoomCategory[]>;
  deleteCategory(parentId: string, categoryId: string): SubRoomResult<readonly SubRoomCategory[]>;
  policy(parentId: string): SubRoomResult<SubRoomPolicy>;
  changePolicy(parentId: string, settings: SubRoomPolicySettings): SubRoomResult<SubRoomPolicy>;
};
