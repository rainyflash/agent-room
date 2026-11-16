import { z } from 'zod';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type { Result } from '@/shared/result';

/**
 * 房间的置顶（`specs/redesign/brief-and-pins.md`“置顶”）：房主和管理员把房间里已经有的一句挂到对话上方，
 * 放当前的事，一间房最多 5 条，新的在最前面。存在 Matrix 标准的 `m.room.pinned_events` 里，由控制面以
 * 应用服务写（私人房间和公开房间的 `state_default` 是管理员那一级，只有应用服务够得着），谁、什么时候
 * 置顶的另记在我们自己的字段 {@link roomPinsField} 里。只记事件 ID，不记那句写了什么。
 */
export const roomPinsEventType = 'm.room.pinned_events';

/** `m.room.pinned_events` 里我们自己的那一段。 */
export const roomPinsField = 'io.github.rainyflash.agentroom.pins';

/** 一间房最多置顶几条，和控制面一样。 */
export const ROOM_PINS_MAX = 5;

/** 置顶的一句：哪一句（Matrix 事件 ID）、谁置顶的、什么时候。 */
export type RoomPin = {
  readonly eventId: string;
  readonly pinnedBy: string;
  readonly pinnedAtUnixMs: number;
};

/** 一间房此刻的置顶，新的在最前面；还有最近一次是谁、什么时候改的（看没看过按它算）。 */
export type RoomPins = {
  readonly entries: readonly RoomPin[];
  readonly updatedAtUnixMs: number;
  readonly updatedBy: string;
};

export type RoomPinsGateway = {
  /**
   * 置顶或取消置顶一句（Matrix 事件 ID），回改完以后的整份，新的在最前面。只有房主和管理员能改，
   * 房间要在使用中；满了回 `room_pins.full`。
   */
  updatePins(
    catalogId: string,
    eventId: string,
    pinned: boolean,
  ): Promise<Result<readonly RoomPin[], PrivateRoomFailure>>;
};

export const roomPinsResponseSchema = z.object({
  pinned: z
    .array(
      z.object({
        eventId: z.string(),
        pinnedBy: z.string(),
        pinnedAtUnixMs: z.number().int().nonnegative(),
      }),
    )
    .max(ROOM_PINS_MAX),
});

/** 置顶改了一次：置顶了一句，或者取消了一条。对话里按时间写一行，点了跳到那一句。 */
export type RoomPinsChangeKind = 'pinned' | 'unpinned';

const eventIdSchema = z
  .string()
  .min(2)
  .max(255)
  .regex(/^\$[^\s]+$/u);
const matrixUserIdSchema = z
  .string()
  .min(4)
  .max(255)
  .regex(/^@[^\s:]+:\S+$/u);

const pinsFieldSchema = z.looseObject({
  entries: z
    .array(
      z.looseObject({
        eventId: eventIdSchema,
        pinnedBy: matrixUserIdSchema,
        pinnedAt: z.iso.datetime({ offset: true }),
      }),
    )
    .max(50),
  updatedBy: matrixUserIdSchema,
  updatedAt: z.iso.datetime({ offset: true }),
  change: z.looseObject({
    kind: z.enum(['pinned', 'unpinned']),
    eventId: eventIdSchema,
  }),
});

const pinnedEventsSchema = z.looseObject({
  pinned: z.array(z.string()).max(50),
  [roomPinsField]: pinsFieldSchema,
});

/**
 * 房间状态里的置顶。标准的 `pinned` 列表说了算，每一句谁、什么时候置顶的照我们的字段补上；我们的字段里
 * 没记的那一句（不是我们写的）不显示。读不出来的、一条都没有的都当没有置顶。
 */
export function parseRoomPins(content: unknown): RoomPins | undefined {
  const parsed = pinnedEventsSchema.safeParse(content);
  if (!parsed.success) return undefined;
  const field = parsed.data[roomPinsField];
  const pinned = new Set(parsed.data.pinned);
  const seen = new Set<string>();
  const entries: RoomPin[] = [];
  for (const entry of field.entries) {
    if (!pinned.has(entry.eventId) || seen.has(entry.eventId)) continue;
    seen.add(entry.eventId);
    entries.push(
      Object.freeze({
        eventId: entry.eventId,
        pinnedBy: entry.pinnedBy,
        pinnedAtUnixMs: Date.parse(entry.pinnedAt),
      }),
    );
  }
  if (entries.length === 0) return undefined;
  return Object.freeze({
    entries: Object.freeze(entries),
    updatedAtUnixMs: Date.parse(field.updatedAt),
    updatedBy: field.updatedBy,
  });
}

/**
 * 这一条置顶的状态事件是一次什么改动：置顶还是取消、哪一句、谁改的。读不出来的不算一次改动。
 * 每一次改动本来就在那条状态事件里，对话里那几行就是记录，不另存。
 */
export function parseRoomPinsChange(content: unknown):
  | {
      readonly kind: RoomPinsChangeKind;
      readonly eventId: string;
      readonly updatedBy: string;
    }
  | undefined {
  const parsed = pinnedEventsSchema.safeParse(content);
  if (!parsed.success) return undefined;
  const field = parsed.data[roomPinsField];
  return {
    kind: field.change.kind,
    eventId: field.change.eventId,
    updatedBy: field.updatedBy,
  };
}

/**
 * 看过哪一次置顶，记在个人工作区的 `acknowledged` 里的键（值是看过的那一次改的时间），和须知的
 * `brief:<房间>` 一样。键最长 512，房间 ID 长到放不下时返回 null，只记在这个浏览器里。
 */
export function pinsSeenKey(roomId: string): string | null {
  const key = `pins:${roomId}`;
  return key.length > 512 ? null : key;
}
