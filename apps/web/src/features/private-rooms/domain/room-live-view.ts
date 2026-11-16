import { z } from 'zod';

import type { Result } from '@/shared/result';
import type { PrivateRoomFailure } from './private-room';

/**
 * 分享页放实况（`specs/redesign/project-rooms.md`“分享页放实况”，3P-6）：房主或管理员打开以后，不登录的人在
 * 分享页上看得到这间房主房间里打开以后说的话（最近 20 句）。只放公开、没关、没被平台拿掉、不加密、直接进的
 * 主房间。
 *
 * 开关是新接口（3P-6a，`GET`/`PUT /private-rooms/{c}/live-view`），回答单独严格校验，不往公开设置、房间的
 * 回答里加字段：那两份网页上也是 `.strict()`，旧版桌面端多看到一个字段整页读不出来。
 */

/** 此刻不满足的几样，服务器按这个先后检查（控制面 `LiveViewCondition`）。 */
export const liveViewConditions = [
  'sub_room',
  'closed',
  'not_listed',
  'platform_hold',
  'encrypted',
  'join_mode',
] as const;

export type LiveViewCondition = (typeof liveViewConditions)[number];

export function isLiveViewCondition(value: string | undefined): value is LiveViewCondition {
  return value !== undefined && (liveViewConditions as readonly string[]).includes(value);
}

const unixMillisSchema = z.number().int().nonnegative();

export const roomLiveViewSchema = z
  .object({
    /** 房主或管理员开着。 */
    enabled: z.boolean(),
    /** 开的那一刻，只放这以后说的话；关着时是 `null`。 */
    enabledAtUnixMs: unixMillisSchema.nullable(),
    /** 分享页此刻放着：开着，而且几样都满足。 */
    live: z.boolean(),
    unmetConditions: z.array(z.enum(liveViewConditions)),
    /** 问的人能不能开关（有“管理”）。 */
    canChange: z.boolean(),
  })
  .strict();

export type RoomLiveView = z.infer<typeof roomLiveViewSchema>;

type Answer<T> = Promise<Result<T, PrivateRoomFailure>>;

export type RoomLiveViewGateway = {
  /** Any joined member can read it; `room_live.not_found` when the room is gone or you aren't in it. */
  liveView(catalogId: string): Answer<RoomLiveView>;
  /**
   * The owner and admins only (`room_live.forbidden` otherwise). Turning it on fails with
   * `room_live.not_eligible` and the first unmet condition in `PrivateRoomFailure.condition`.
   */
  updateLiveView(catalogId: string, enabled: boolean): Answer<RoomLiveView>;
};
