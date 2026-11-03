import { z } from 'zod';

import type { PrivateRoomFailure } from '@/features/private-rooms/domain/private-room';
import type { Result } from '@/shared/result';

/**
 * 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：房主和管理员写的一段纯文本，挂在对话上方，
 * 每间房一条，新写的盖掉旧的，清空就是撤掉。由控制面以应用服务写进房间状态；私人房间和公开房间的
 * `state_default` 是管理员那一级，只有应用服务够得着，所以房间状态里有的就是控制面写的。
 */
export const roomNoticeEventType = 'io.github.rainyflash.agentroom.room.notice.v1';

/** 公告最多写多少个字，按 Unicode 字符数，和控制面一样。 */
export const ROOM_NOTICE_MAX_CHARACTERS = 2_000;

export type RoomNotice = {
  readonly text: string;
  /** 改的人在 Matrix 上的 ID。 */
  readonly updatedBy: string;
  readonly updatedAtUnixMs: number;
};

/** 控制面写进去的那一版。 */
export type WrittenRoomNotice = {
  readonly text: string;
  readonly updatedAtUnixMs: number;
};

export type RoomNoticeGateway = {
  /** 改公告；空的就是撤掉。只有房主和管理员能改，房间要在使用中。 */
  updateNotice(
    catalogId: string,
    text: string,
  ): Promise<Result<WrittenRoomNotice, PrivateRoomFailure>>;
};

export const writtenRoomNoticeSchema = z.object({
  text: z.string(),
  updatedAtUnixMs: z.number().int().nonnegative(),
});

const noticeStateSchema = z.looseObject({
  eventType: z.literal(roomNoticeEventType),
  text: z.string(),
  updatedBy: z
    .string()
    .min(4)
    .max(255)
    .regex(/^@[^\s:]+:\S+$/u),
  updatedAt: z.iso.datetime({ offset: true }),
});

/**
 * 房间状态里的公告。读不出来的、撤掉了的（正文是空的）都当没有公告；超长的也不认，控制面不会写出这样的。
 */
export function parseRoomNotice(content: unknown): RoomNotice | undefined {
  const parsed = noticeStateSchema.safeParse(content);
  if (!parsed.success) return undefined;
  const text = parsed.data.text.trim();
  if (text === '' || characterCount(text) > ROOM_NOTICE_MAX_CHARACTERS) return undefined;
  return Object.freeze({
    text,
    updatedAtUnixMs: Date.parse(parsed.data.updatedAt),
    updatedBy: parsed.data.updatedBy,
  });
}

/** 公告的一次改动：写了新的，或者撤掉了（正文是空的）。 */
export type RoomNoticeChangeKind = 'updated' | 'removed';

/** 状态事件里这次是写还是撤、谁改的。读不出来的、超长的不算一次改动。 */
export function parseRoomNoticeChange(
  content: unknown,
): { readonly kind: RoomNoticeChangeKind; readonly updatedBy: string } | undefined {
  const parsed = noticeStateSchema.safeParse(content);
  if (!parsed.success) return undefined;
  const text = parsed.data.text.trim();
  if (characterCount(text) > ROOM_NOTICE_MAX_CHARACTERS) return undefined;
  return { kind: text === '' ? 'removed' : 'updated', updatedBy: parsed.data.updatedBy };
}

export type RoomNoticeProblem = 'control_character' | 'too_long';

/** 按控制面的规则先看一遍，免得发出去才被拒：去掉首尾空白以后最多 2000 个字，只能带换行和制表符。 */
export function roomNoticeProblem(text: string): RoomNoticeProblem | null {
  const trimmed = normalizeRoomNotice(text);
  if (characterCount(trimmed) > ROOM_NOTICE_MAX_CHARACTERS) return 'too_long';
  // eslint-disable-next-line no-control-regex -- 要找的就是控制字符。
  return /[\u0000-\u0008\u000B-\u001F\u007F-\u009F]/u.test(trimmed) ? 'control_character' : null;
}

/** 发出去的样子：回车换成换行，去掉首尾空白。 */
export function normalizeRoomNotice(text: string): string {
  return text.replace(/\r\n?/gu, '\n').trim();
}

/** 按 Unicode 字符数，和服务器（Rust 的 `chars().count()`）一样，不按字形。 */
export function characterCount(text: string): number {
  // eslint-disable-next-line @typescript-eslint/no-misused-spread -- 要的就是逐个 Unicode 字符。
  return [...text].length;
}
