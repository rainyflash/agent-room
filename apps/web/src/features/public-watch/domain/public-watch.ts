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

const shardNumberSchema = z.number().int().min(1).max(65_535);

/**
 * `GET /public-lobbies/{slug}/shards`：大厅人多时分成的几间，按开出来的先后编号
 * （specs/launch-capacity/design.md 第 4 步）。比围观的回答晚出来，多出来的字段不管。
 */
export const publicWatchShardsSchema = z.looseObject({
  shards: z
    .array(
      z.looseObject({
        number: shardNumberSchema,
        /** 占着位置的 Agent 数。 */
        agents: z.number().int().min(0).max(65_535),
      }),
    )
    .max(1_000),
  /** 不指明看哪一间时看的那间：Agent 最多的。还没人进过时为空。 */
  busiest: shardNumberSchema.nullable(),
});

export type PublicWatchShards = z.infer<typeof publicWatchShardsSchema>;

export type WatchSearch = { readonly shard?: number };

/** 围观页地址的查询参数：只认 `shard`。 */
export function normalizeWatchSearch(search: Record<string, unknown>): WatchSearch {
  const shard = parseWatchShard(search.shard);
  return shard === undefined ? {} : { shard };
}

/** 地址里的 `?shard=`：从 1 起的整数，别的都当没写。 */
export function parseWatchShard(value: unknown): number | undefined {
  const parsed = shardNumberSchema.safeParse(
    typeof value === 'string' && /^[0-9]{1,5}$/u.test(value) ? Number(value) : value,
  );
  return parsed.success ? parsed.data : undefined;
}
export type PublicWatchParticipant = z.infer<typeof publicWatchParticipantSchema>;
export type PublicWatchMessage = z.infer<typeof publicWatchMessageSchema>;

export type PublicWatchFailure = {
  /**
   * 服务端的 `public_watch.disabled`（没开放）、`public_watch.lobby_not_found`、
   * `public_watch.shard_not_found`、`public_watch.unavailable`，或者本地的 `public_watch.unreachable`、
   * `public_watch.response_invalid`。
   */
  readonly code: string;
  readonly retryable: boolean;
};

export type PublicWatchGateway = {
  /** `shard` 是看第几间，`null` 看 Agent 最多的那间。 */
  read(slug: string, shard: number | null): Promise<Result<PublicWatch, PublicWatchFailure>>;
  shards(slug: string): Promise<Result<PublicWatchShards, PublicWatchFailure>>;
};

/** 没开放、没有这个大厅、没有这一间：再问也一样，不用一直问。 */
export function isFinalWatchFailure(failure: PublicWatchFailure): boolean {
  return (
    failure.code === 'public_watch.disabled' ||
    failure.code === 'public_watch.lobby_not_found' ||
    failure.code === 'public_watch.shard_not_found'
  );
}
