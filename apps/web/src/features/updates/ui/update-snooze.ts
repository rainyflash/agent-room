import { z } from 'zod';

const snoozeKey = 'agent-room.desktop-update.snoozed';

/** 点了“稍后”，同一个版本多久以后再提醒。 */
export const updateSnoozeMs = 24 * 60 * 60 * 1_000;

const updateSnoozeSchema = z
  .object({
    version: z.string().min(1).max(64),
    untilUnixMs: z.number().int().nonnegative(),
  })
  .strict();
export type UpdateSnooze = z.infer<typeof updateSnoozeSchema>;

/** 桌面端网页的本地存储；读不到（被禁用、隐私模式）时为 null，提醒照常出。 */
export function browserStorageOrNull(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function readUpdateSnooze(storage: Pick<Storage, 'getItem'> | null): UpdateSnooze | null {
  try {
    const raw = storage?.getItem(snoozeKey);
    if (raw == null) return null;
    const parsed = updateSnoozeSchema.safeParse(JSON.parse(raw));
    return parsed.success ? parsed.data : null;
  } catch {
    return null;
  }
}

/** 记不住也不要紧：最多下次打开时再提醒一回。 */
export function writeUpdateSnooze(
  storage: Pick<Storage, 'setItem' | 'removeItem'> | null,
  snooze: UpdateSnooze | null,
): void {
  try {
    if (snooze === null) storage?.removeItem(snoozeKey);
    else storage?.setItem(snoozeKey, JSON.stringify(snooze));
  } catch {
    // 本地存储写不进去时这次照样收起，只是重开以后会再提醒。
  }
}

/**
 * 这个版本还要再等多久才提醒：同一个版本“稍后”以后 24 小时，更新的版本马上提醒。电脑时钟往回调过
 * 也最多等 24 小时。
 */
export function snoozeRemainingMs(
  snooze: UpdateSnooze | null,
  version: string,
  nowUnixMs: number,
): number {
  if (snooze?.version !== version) return 0;
  return Math.min(updateSnoozeMs, Math.max(0, snooze.untilUnixMs - nowUnixMs));
}
