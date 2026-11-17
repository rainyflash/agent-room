import { z } from 'zod';

import type { RoomMessageSignal } from '@/features/messages/domain/message';
import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type { Result } from '@/shared/result';

/**
 * 分享一段对话（`specs/redesign/project-rooms.md`“分享一段对话”，3P-7）：放着实况的房间里，连着的最多 30
 * 句分享成一个不用登录就打得开的链接（`/r/<分享码>/s/<段码>`）。服务器只记是哪几句、谁什么时候分享的，
 * 不存话；打开时照实况那一套重新读，撤回、隐藏的不再显示。
 *
 * 接口都是新的（3P-7a-1，`/private-rooms/{c}/excerpts`），回答单独严格校验，不往房间、公开设置、实况开关的
 * 回答里加字段：那几份网页上都是 `.strict()`，旧版桌面端多看到一个字段整页读不出来。
 */

/** 一段最多几句，和控制面 `MAX_EXCERPT_MESSAGES` 一样。 */
export const EXCERPT_MAX_MESSAGES = 30;
/** 预览里每句给多少个字，和分享出去的那一页一样（实况也是这么多）。 */
export const EXCERPT_PREVIEW_CHARS = 280;

/** 8 位大小写字母和数字（和分享码同一套字母，控制面 `RoomExcerptCode`）。 */
const excerptCodePattern = /^[A-Za-z0-9]{8}$/u;
const unixMillisSchema = z.number().int().nonnegative();
const excerptUrlSchema = z.url({ protocol: /^https?$/u });

export const sharedExcerptSchema = z
  .object({
    code: z.string().regex(excerptCodePattern),
    /** 服务器拼好的整条链接（裸域上的 `/r/<分享码>/s/<段码>`）：网页不知道裸域在哪。 */
    url: excerptUrlSchema,
  })
  .strict();

export type SharedExcerpt = z.infer<typeof sharedExcerptSchema>;

export const roomExcerptSchema = z
  .object({
    code: z.string().regex(excerptCodePattern),
    /** 房间没有分享码时是 `null`（放实况的房间都公开着，不会有）。 */
    url: excerptUrlSchema.nullable(),
    messageCount: z.number().int().min(1).max(EXCERPT_MAX_MESSAGES),
    firstMessageAtUnixMs: unixMillisSchema,
    sharedAtUnixMs: unixMillisSchema,
    /** 是不是你分享的。房主和管理员看得到别人分享的，但看不出是谁。 */
    mine: z.boolean(),
  })
  .strict();

export type RoomExcerpt = z.infer<typeof roomExcerptSchema>;

export const roomExcerptListSchema = z
  .object({ excerpts: z.array(roomExcerptSchema).max(200) })
  .strict();

/** 选的那一段：开头和结尾那一句的 Matrix 事件 ID。 */
export type ExcerptBounds = {
  readonly firstEventId: string;
  readonly lastEventId: string;
};

type Answer<T> = Promise<Result<T, PrivateRoomFailure>>;

export type RoomExcerptGateway = {
  /**
   * Any joined member of a room with the live view on. Fails with `room_excerpt.not_live`,
   * `room_excerpt.too_long`, `room_excerpt.invalid_range` (the reason in `PrivateRoomFailure.reason`)
   * or `room_excerpt.rate_limited` (with `retryAfterSeconds`).
   */
  share(catalogId: string, bounds: ExcerptBounds): Answer<SharedExcerpt>;
  /** Your own; the owner and admins get every one in the room, newest first. */
  list(catalogId: string): Answer<readonly RoomExcerpt[]>;
  /** The one who shared it, the owner and admins. */
  remove(catalogId: string, code: string): Answer<void>;
};

/**
 * 这一句能不能分享：真的发出去了的聊天消息（Matrix 事件 ID 以 `$` 开头，还在发的本地回显不算），开了实况
 * 以后说的。系统那几行（改须知、置顶、换房主）本来就不是消息。
 */
export function excerptShareable(message: RoomMessageSignal, liveSinceUnixMs: number): boolean {
  return (
    message.lifecycle === 'active' &&
    message.preview?.conversation !== undefined &&
    message.matrixEventId.startsWith('$') &&
    message.serverTimestamp >= liveSinceUnixMs
  );
}

/** 选中的一段，按对话里的消息 ID 记：开头和结尾。 */
export type ExcerptRange = {
  readonly startId: string;
  readonly endId: string;
};

/** 选中的那几句，照对话里的先后；开头或结尾已经不在对话里了（撤回了）是 `null`。 */
export function excerptMessages(
  timeline: readonly RoomMessageSignal[],
  range: ExcerptRange,
): readonly RoomMessageSignal[] | null {
  const start = timeline.findIndex((message) => message.messageId === range.startId);
  const end = timeline.findIndex((message) => message.messageId === range.endId);
  if (start < 0 || end < start) return null;
  return timeline.slice(start, end + 1);
}

export type ExcerptPick = {
  readonly range: ExcerptRange;
  /** 这一下没改成：选到那一句就超过 30 句了。 */
  readonly tooLong: boolean;
};

/**
 * 选的时候点了一句：在开头后面的成了结尾（在已选的中间就缩到那儿）；在开头前面的成了开头，结尾不动；
 * 点开头那一句只留它自己。超过 30 句的不改，说为什么。
 */
export function pickExcerptMessage(
  timeline: readonly RoomMessageSignal[],
  range: ExcerptRange,
  messageId: string,
): ExcerptPick {
  const start = timeline.findIndex((message) => message.messageId === range.startId);
  const end = timeline.findIndex((message) => message.messageId === range.endId);
  const picked = timeline.findIndex((message) => message.messageId === messageId);
  if (start < 0 || end < start || picked < 0) return { range, tooLong: false };
  const next =
    picked >= start
      ? { startId: range.startId, endId: messageId }
      : { ...range, startId: messageId };
  const first = picked >= start ? start : picked;
  const last = picked >= start ? picked : end;
  return last - first + 1 > EXCERPT_MAX_MESSAGES
    ? { range, tooLong: true }
    : { range: next, tooLong: false };
}

/** 一句在选中的那一段里排在哪：头一句、中间、最后一句、只有它；不在里面是 `null`。 */
export type ExcerptPlace = 'first' | 'middle' | 'last' | 'only';

export function excerptPlaces(
  selected: readonly RoomMessageSignal[],
): ReadonlyMap<string, ExcerptPlace> {
  return new Map(
    selected.map((message, index) => [
      message.messageId,
      selected.length === 1
        ? 'only'
        : index === 0
          ? 'first'
          : index === selected.length - 1
            ? 'last'
            : 'middle',
    ]),
  );
}

/** 预览里一句的样子，和分享出去的那一页一样：不公开的只说一句，正文开头 280 个字，附件只说有。 */
export type ExcerptPreviewLine =
  | { readonly kind: 'withheld' }
  | {
      readonly kind: 'text';
      readonly text: string;
      readonly truncated: boolean;
      readonly attachment: boolean;
    };

export function excerptPreviewLine(message: RoomMessageSignal): ExcerptPreviewLine {
  if (message.preview?.sensitivity !== 'normal') return { kind: 'withheld' };
  const chat = message.preview.conversation;
  const attachment = chat?.attachmentName !== undefined;
  // 只发了一个文件时正文就是文件名：只说发了一个文件。
  const raw =
    chat === undefined || (attachment && chat.text === chat.attachmentName) ? '' : chat.text;
  const characters = Array.from(raw);
  const truncated = characters.length > EXCERPT_PREVIEW_CHARS;
  return {
    kind: 'text',
    text: truncated ? characters.slice(0, EXCERPT_PREVIEW_CHARS).join('') : raw,
    truncated,
    attachment,
  };
}

/** 分享没成、删掉没成时，人能做什么（决定说哪一句、给不给“再试一次”）。 */
export type ExcerptFailureKind =
  | 'not_live'
  | 'too_long'
  | 'before_live'
  | 'range_not_found'
  | 'rate_limited'
  | 'not_found'
  | 'forbidden'
  | 'unavailable'
  | 'invalid'
  | 'invalid_origin'
  | 'unreachable'
  | 'other';

export function excerptFailureKind(failure: PrivateRoomFailure): ExcerptFailureKind {
  switch (failure.code) {
    case 'room_excerpt.not_live':
      return 'not_live';
    case 'room_excerpt.too_long':
      return 'too_long';
    case 'room_excerpt.invalid_range':
      return failure.reason === 'before_live' ? 'before_live' : 'range_not_found';
    case 'room_excerpt.rate_limited':
      return 'rate_limited';
    case 'room_excerpt.not_found':
      return 'not_found';
    case 'room_excerpt.forbidden':
      return 'forbidden';
    case 'room_excerpt.dependency_unavailable':
    case 'room_excerpt.internal':
      return 'unavailable';
    case 'room_excerpt.invalid':
    case 'room_excerpt.invalid_resource':
    case 'private_room.invalid_response':
      return 'invalid';
    case 'room_excerpt.invalid_origin':
      return 'invalid_origin';
    case 'private_room.unreachable':
      return 'unreachable';
    default:
      return 'other';
  }
}

/** 等一会儿再试有用的几种：给“再试一次”。太多了（429）再试也没用，要等。 */
export function excerptFailureRetryable(kind: ExcerptFailureKind): boolean {
  return kind === 'unavailable' || kind === 'unreachable' || kind === 'other';
}

/** 429 时还要等几分钟，向上取整；服务器没说就是 `null`。 */
export function excerptRetryMinutes(failure: PrivateRoomFailure): number | null {
  return failure.retryAfterSeconds === undefined
    ? null
    : Math.max(1, Math.ceil(failure.retryAfterSeconds / 60));
}
