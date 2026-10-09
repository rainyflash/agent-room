import { z } from 'zod';

import { lobbyAgentStatuses } from '@/features/lobby/domain/lobby';
import type { Result } from '@/shared/result';
import { uuidV7Schema } from '@/shared/validation/identifiers';

/**
 * 不登录也能看公共大厅（specs/public-lobby-watch/design.md）：`GET /public-lobbies/{slug}/watch`
 * 的回答。服务端只给公开展示得了的：人和消息是快照内部的编号，没有 Matrix 的房间和用户。
 *
 * 新接口，按自己的形状严格校验：服务端加字段之前，先发能接受新字段的网页。
 */

/** `/watch` 不带 slug 时问的是默认大厅。 */
export const defaultWatchSlug = 'default';

/** 公共大厅的 slug 规则，和控制面的 `RoomSlug` 一样。 */
const slugPattern = /^[a-z0-9][a-z0-9-]{0,62}$/u;

export function isWatchSlug(value: string): boolean {
  return slugPattern.test(value);
}

/** 快照内部的编号：同一个控制面进程里不变，重启以后全换，所以只在一份快照里用。 */
const watchKeySchema = z.string().min(1).max(64);

export const publicWatchParticipantSchema = z
  .object({
    key: watchKeySchema,
    name: z.string().min(1).max(256),
    kind: z.enum(['person', 'agent', 'networkAgent']),
    /** 只有 Agent 有在线状态；人一律是 false。 */
    online: z.boolean(),
    /** 在线的 Agent 自己报的粗略状态；不在线的和人是 null。 */
    status: z.enum(lobbyAgentStatuses).nullable(),
  })
  .strict();

export const publicWatchMessageSchema = z
  .object({
    key: watchKeySchema,
    author: watchKeySchema,
    /** 最多 1000 个字符；不公开的消息是空的。 */
    text: z.string().max(4_000),
    truncated: z.boolean(),
    /** 敏感或受限的消息：只说有一条。 */
    withheld: z.boolean(),
    /** 带了文件：只说带了，不给文件名和内容。 */
    attachment: z.boolean(),
    edited: z.boolean(),
    /** 回复的那条的编号；那条可能不在这一页里。 */
    replyTo: watchKeySchema.nullable(),
    sentAtUnixMs: z.number().int().nonnegative(),
  })
  .strict();

export const publicWatchSchema = z
  .object({
    schemaVersion: z.literal(1),
    lobby: z
      .object({
        catalogId: uuidV7Schema,
        name: z.string().trim().min(1).max(160),
        slug: z.string().regex(slugPattern),
      })
      .strict(),
    participants: z.array(publicWatchParticipantSchema).max(200),
    messages: z.array(publicWatchMessageSchema).max(50),
    updatedAtUnixMs: z.number().int().nonnegative(),
  })
  .strict();

export type PublicWatch = z.infer<typeof publicWatchSchema>;
export type PublicWatchParticipant = z.infer<typeof publicWatchParticipantSchema>;
export type PublicWatchMessage = z.infer<typeof publicWatchMessageSchema>;

export type PublicWatchFailure = {
  /**
   * 服务端的 `public_watch.disabled`（没开放）、`public_watch.lobby_not_found`、
   * `public_watch.unavailable`，或者本地的 `public_watch.unreachable`、`public_watch.response_invalid`。
   */
  readonly code: string;
  readonly retryable: boolean;
};

export type PublicWatchGateway = {
  read(slug: string): Promise<Result<PublicWatch, PublicWatchFailure>>;
};

/** 没开放、没有这个大厅：再问也一样，不用一直问。 */
export function isFinalWatchFailure(failure: PublicWatchFailure): boolean {
  return (
    failure.code === 'public_watch.disabled' || failure.code === 'public_watch.lobby_not_found'
  );
}
