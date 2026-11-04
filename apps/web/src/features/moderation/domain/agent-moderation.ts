import { z } from 'zod';

import type { ModerationAction, ModerationFailure } from '@/features/moderation/domain/moderation';
import type { Result } from '@/shared/result';

/**
 * 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：房主和管理员把房间交给自己的
 * Agent 管，台账里写着哪条是哪个 Agent 照规则做的。都是单独的接口，不加进 `moderationActionSchema`：旧版网页
 * 对治理动作严格校验，多一个字段整个台账都读不出来。
 */
export const roomModerationPowers = ['mute', 'kick', 'ban', 'unban'] as const;

export type RoomModerationPower = (typeof roomModerationPowers)[number];

/** 和服务器一样：规则最多 2000 个字，授权最短一分钟、最长 30 天。 */
export const MAX_MODERATION_RULES_CHARACTERS = 2_000;
export const MIN_MODERATION_GRANT_SECONDS = 60;
export const MAX_MODERATION_GRANT_SECONDS = 30 * 24 * 60 * 60;

const uuidV7Schema = z
  .string()
  .regex(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u);
const timestampSchema = z.number().int().nonnegative();

export const roomModerationGrantSchema = z
  .object({
    actions: z.array(z.enum(roomModerationPowers)).min(1).max(roomModerationPowers.length),
    agentId: uuidV7Schema,
    agentName: z.string().min(1).max(200),
    createdAtUnixMs: timestampSchema,
    expiresAtUnixMs: timestampSchema,
    /** 给的人的账户 ID，按成员名单换成昵称。 */
    grantedBy: uuidV7Schema,
    id: uuidV7Schema,
    rules: z
      .string()
      .min(1)
      .max(MAX_MODERATION_RULES_CHARACTERS * 4),
  })
  .strict();

export const roomModerationGrantListSchema = z
  .object({ grants: z.array(roomModerationGrantSchema).max(1_000).readonly() })
  .strict();

/** Agent 做的一条（或撤回的一条，`actionId` 是被撤回的那条）和它写的理由。 */
export const agentModerationRecordSchema = z
  .object({
    actionId: uuidV7Schema,
    agentId: uuidV7Schema,
    agentName: z.string().min(1).max(200),
    grantId: uuidV7Schema,
    note: z.string().min(1).max(4_000),
  })
  .strict();

export const agentModerationLedgerSchema = z
  .object({
    actions: z.array(agentModerationRecordSchema).max(1_000).readonly(),
    reversals: z.array(agentModerationRecordSchema).max(1_000).readonly(),
  })
  .strict();

export const grantReversalSchema = z
  .object({
    failed: z
      .array(z.object({ actionId: uuidV7Schema, code: z.string().min(1).max(128) }).strict())
      .max(1_000)
      .readonly(),
    reversed: z.array(uuidV7Schema).max(1_000).readonly(),
  })
  .strict();

export const grantRoomModerationInputSchema = z
  .object({
    actions: z.array(z.enum(roomModerationPowers)).min(1).max(roomModerationPowers.length),
    agentId: uuidV7Schema,
    lifetimeSeconds: z
      .number()
      .int()
      .min(MIN_MODERATION_GRANT_SECONDS)
      .max(MAX_MODERATION_GRANT_SECONDS),
    rules: z
      .string()
      .trim()
      .refine(
        (rules) => rules.length > 0 && Array.from(rules).length <= MAX_MODERATION_RULES_CHARACTERS,
        'rules',
      ),
  })
  .strict();

export type RoomModerationGrant = z.output<typeof roomModerationGrantSchema>;
export type AgentModerationRecord = z.output<typeof agentModerationRecordSchema>;
export type AgentModerationLedger = z.output<typeof agentModerationLedgerSchema>;
export type GrantReversal = z.output<typeof grantReversalSchema>;
export type GrantRoomModerationInput = z.input<typeof grantRoomModerationInputSchema>;

export type AgentModerationGateway = {
  /** Grants in use in this private room; owner and admins only. */
  listGrants(catalogId: string): Promise<Result<readonly RoomModerationGrant[], ModerationFailure>>;
  /** Gives one of your own agents a grant; replaces its previous one. Needs a recent sign-in. */
  grant(
    catalogId: string,
    input: GrantRoomModerationInput,
  ): Promise<Result<RoomModerationGrant, ModerationFailure>>;
  /** Revokes a grant. What the agent already did stays in effect. */
  revokeGrant(catalogId: string, grantId: string): Promise<Result<void, ModerationFailure>>;
  /** Which ledger actions an agent took or undid, with the note it wrote. */
  listAgentActions(catalogId: string): Promise<Result<AgentModerationLedger, ModerationFailure>>;
  /**
   * Undoes every action still in effect that an agent took under this grant, even after the grant was
   * revoked or expired. One failure does not stop the others; trying again only retries the failed ones.
   */
  reverseGrant(
    catalogId: string,
    grantId: string,
  ): Promise<Result<GrantReversal, ModerationFailure>>;
};

/** 一份授权下 Agent 做的、此刻还生效着的动作：一次全撤按它列。 */
export type GrantInEffect = {
  readonly actionIds: readonly string[];
  readonly agentName: string;
  readonly grantId: string;
};

/**
 * 按授权把 Agent 做的、还生效着（`applied`）的动作归到一起，照台账的先后排。撤了的、到期解除的、人做的都不算；
 * 授权已经撤了、到了期的也照样列，它之前做的照旧生效，要撤得人来撤。
 */
export function grantsInEffect(
  actions: readonly ModerationAction[],
  ledger: AgentModerationLedger,
): readonly GrantInEffect[] {
  const applied = new Set(
    actions.filter((action) => action.status === 'applied').map((action) => action.actionId),
  );
  const byGrant = new Map<string, { agentName: string; actionIds: string[] }>();
  for (const record of ledger.actions) {
    if (!applied.has(record.actionId)) continue;
    const entry = byGrant.get(record.grantId) ?? { actionIds: [], agentName: record.agentName };
    entry.actionIds.push(record.actionId);
    byGrant.set(record.grantId, entry);
  }
  return [...byGrant].map(([grantId, entry]) => ({ grantId, ...entry }));
}
