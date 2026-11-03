import { z } from 'zod';

import type { Result } from '@/shared/result';
import { uuidV7Schema } from '@/shared/validation/identifiers';

/**
 * 发现页（`specs/public-rooms/design.md`“发现页”）：公开房间的目录和一间房的详情。不登录也能读；
 * 带着有效的登录时每间房多一个 `viewer`，说你在不在里面。
 */
export const exploreJoinModes = ['open', 'request', 'invite_only'] as const;
export const exploreViewerStates = ['joined', 'invited', 'pending', 'banned'] as const;

const unixMillisSchema = z.number().int().nonnegative();
const countSchema = z.number().int().nonnegative();

export const exploreShareCodeSchema = z.string().regex(/^[A-Za-z0-9]{6}$/u);

export const exploreRoomSchema = z
  .object({
    agentCount: countSchema,
    catalogId: uuidV7Schema,
    createdAtUnixMs: unixMillisSchema,
    description: z.string(),
    endToEndEncrypted: z.boolean(),
    joinMode: z.enum(exploreJoinModes),
    lastMessageAtUnixMs: unixMillisSchema.nullable(),
    memberCount: countSchema,
    name: z.string().min(1),
    ownerName: z.string(),
    shareCode: exploreShareCodeSchema,
    shareUrl: z.url({ protocol: /^https?$/u }),
    viewer: z.enum(exploreViewerStates).nullable(),
  })
  .strict();

export const explorePageSchema = z
  .object({
    nextCursor: z
      .string()
      .regex(/^\d{1,9}$/u)
      .optional(),
    rooms: z.array(exploreRoomSchema),
  })
  .strict();

export const exploreRoomDetailSchema = exploreRoomSchema
  .extend({
    admins: z.array(z.string()),
    /** 现在几个在线；问不到时是 `null`。`atLeast` 是没问全，可能更多。 */
    online: z.object({ atLeast: z.boolean(), count: countSchema }).strict().nullable(),
  })
  .strict();

export type ExploreJoinMode = (typeof exploreJoinModes)[number];
export type ExploreViewerState = (typeof exploreViewerStates)[number];
export type ExploreRoom = z.infer<typeof exploreRoomSchema>;
export type ExploreRoomDetail = z.infer<typeof exploreRoomDetailSchema>;
export type ExplorePage = {
  readonly nextCursor: string | null;
  readonly rooms: readonly ExploreRoom[];
};

/** 快捷筛选一次只选一个；“全部”是不筛。 */
export const exploreQuickFilters = ['all', 'open', 'request', 'active', 'plain'] as const;
export const exploreSorts = ['active', 'members', 'newest'] as const;
export type ExploreQuickFilter = (typeof exploreQuickFilters)[number];
export type ExploreSort = (typeof exploreSorts)[number];

export type ExploreFilter = {
  /** 搜房间名、简介和房主的昵称，最多 100 个字。 */
  readonly q: string;
  readonly quick: ExploreQuickFilter;
  readonly sort: ExploreSort;
};

export const defaultExploreFilter: ExploreFilter = { q: '', quick: 'all', sort: 'active' };
export const EXPLORE_SEARCH_MAX_LENGTH = 100;

export type ExploreFailure = {
  readonly code: string;
  readonly correlationId?: string;
  readonly retryable: boolean;
  /** 被限流（`public_rooms.rate_limited`）时过多少秒再试。 */
  readonly retryAfterSeconds?: number;
};

export type ExploreGateway = {
  /** One page of public rooms; pass the previous page's `nextCursor` to get the next one. */
  browse(
    filter: ExploreFilter,
    cursor: string | null,
  ): Promise<Result<ExplorePage, ExploreFailure>>;
  /** One room by its share code; a room that is no longer public is `public_room.not_found`. */
  detail(shareCode: string): Promise<Result<ExploreRoomDetail, ExploreFailure>>;
};

/**
 * 发现页地址上的搜索、筛选和排序（`/explore?q=&quick=&sort=`），刷新、分享、后退都还在。默认的不写进地址；
 * 认不出的值当默认的。
 *
 * `join=1` 是从分享页点“加入”过来的（`/explore/<分享码>?join=1`）：没登录的先去登录，`signup=1` 时去注册，
 * 回来以后接着加入（见 `joinContinuationStep`）。
 */
export type ExploreSearch = {
  readonly q?: string;
  readonly quick?: Exclude<ExploreQuickFilter, 'all'>;
  readonly sort?: Exclude<ExploreSort, 'active'>;
  readonly join?: 1;
  readonly signup?: 1;
};

export function normalizeExploreSearch(search: Record<string, unknown>): ExploreSearch {
  const q = typeof search.q === 'string' ? search.q.slice(0, EXPLORE_SEARCH_MAX_LENGTH) : '';
  const quick = exploreQuickFilters.find((value) => value === search.quick && value !== 'all');
  const sort = exploreSorts.find((value) => value === search.sort && value !== 'active');
  const join = isFlag(search.join);
  return {
    ...(q.trim() === '' ? {} : { q }),
    ...(quick === undefined || quick === 'all' ? {} : { quick }),
    ...(sort === undefined || sort === 'active' ? {} : { sort }),
    ...(join ? { join: 1 as const } : {}),
    ...(join && isFlag(search.signup) ? { signup: 1 as const } : {}),
  };
}

/** 路由把 `?join=1` 读成数字 1；手写的 `true`、`"1"` 也认。 */
function isFlag(value: unknown): boolean {
  return value === 1 || value === '1' || value === true || value === 'true';
}

/** 登录、注册完回来接着加入的地址。 */
export function exploreJoinPath(shareCode: string): string {
  return `/explore/${encodeURIComponent(shareCode)}?join=1`;
}

/**
 * 从分享页点“加入”、登录回来以后做什么：已经在里面的直接进房间；直接进的、收到过邀请的加入；要申请的
 * 打开申请框；在等批准、被封禁、只能邀请的什么都不做，详情上自己会说。
 */
export function joinContinuationStep(room: ExploreRoom): 'enter' | 'join' | 'request' | 'none' {
  switch (room.viewer) {
    case 'joined':
      return 'enter';
    case 'invited':
      return 'join';
    case 'pending':
    case 'banned':
      return 'none';
    case null:
      break;
  }
  switch (room.joinMode) {
    case 'open':
      return 'join';
    case 'request':
      return 'request';
    case 'invite_only':
      return 'none';
  }
}

/** 有在等批准的申请时，目录和详情多久重读一次：批准了好马上带他进去。 */
export const JOIN_REQUEST_POLL_MS = 10_000;

export function hasPendingRequest(rooms: readonly ExploreRoom[]): boolean {
  return rooms.some((room) => room.viewer === 'pending');
}

/**
 * 申请被批准了的房间（`specs/public-rooms/design.md`“加入方式”）：批准以房主的名义邀请，所以这一页看到过
 * 在等批准、现在收到了邀请的，就是批准了。`waiting` 记着这一页看到过在等批准的房间，原地更新；撤回了、
 * 被拒了的拿掉，不算批准。没看到过在等批准的邀请不算：别的邀请照旧要人自己点“接受邀请”。
 *
 * 同一间房目录和详情里都有时，看走得远的那个：详情刚重读过，目录可能还是旧的“在等批准”。
 */
export function approvedRequests(
  waiting: Set<string>,
  rooms: readonly ExploreRoom[],
): ExploreRoom[] {
  const latest = new Map<string, ExploreRoom>();
  for (const room of rooms) {
    const seen = latest.get(room.catalogId);
    if (seen === undefined || seen.viewer === 'pending') latest.set(room.catalogId, room);
  }
  const approved: ExploreRoom[] = [];
  for (const room of latest.values()) {
    if (room.viewer === 'pending') waiting.add(room.catalogId);
    else if (waiting.delete(room.catalogId) && room.viewer === 'invited') approved.push(room);
  }
  return approved;
}

export function exploreFilterFromSearch(search: ExploreSearch): ExploreFilter {
  return { q: search.q ?? '', quick: search.quick ?? 'all', sort: search.sort ?? 'active' };
}

export function exploreSearchFromFilter(filter: ExploreFilter): ExploreSearch {
  return normalizeExploreSearch({ q: filter.q, quick: filter.quick, sort: filter.sort });
}

/** 查询参数：快捷筛选换成服务器认的几样，“全部”和默认的排序不带。 */
export function exploreSearchParams(filter: ExploreFilter, cursor: string | null): URLSearchParams {
  const params = new URLSearchParams();
  const q = filter.q.trim();
  if (q !== '') params.set('q', q);
  switch (filter.quick) {
    case 'open':
    case 'request':
      params.set('join', filter.quick);
      break;
    case 'active':
      params.set('active', 'true');
      break;
    case 'plain':
      params.set('encrypted', 'false');
      break;
    case 'all':
      break;
  }
  if (filter.sort !== 'active') params.set('sort', filter.sort);
  if (cursor !== null) params.set('cursor', cursor);
  return params;
}

/** 头像底色按房间号挑：同一间房在哪里都是同一个颜色。 */
export const exploreAvatarTones = ['sunny', 'sky', 'mint', 'coral', 'accent-soft'] as const;
export type ExploreAvatarTone = (typeof exploreAvatarTones)[number];

export function avatarTone(catalogId: string): ExploreAvatarTone {
  let hash = 0;
  for (const character of catalogId) {
    hash = (hash * 31 + (character.codePointAt(0) ?? 0)) >>> 0;
  }
  return exploreAvatarTones[hash % exploreAvatarTones.length] ?? 'sunny';
}

/** 头像上写名字的第一个字：按字形切，带肤色的表情、组合的字母都算一个。 */
export function avatarLetter(name: string): string {
  const segments = new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(name.trim());
  const first = segments[Symbol.iterator]().next();
  return first.done === true ? '#' : first.value.segment.toLocaleUpperCase();
}

/** 一小时内有人说话：和服务器的“一小时内有人说话”筛选同一个标准。 */
export const RECENTLY_ACTIVE_MS = 60 * 60 * 1_000;
