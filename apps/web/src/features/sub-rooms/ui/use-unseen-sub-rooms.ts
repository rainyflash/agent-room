import { useEffect, useState } from 'react';

import type { SubRoomList } from '@/features/private-rooms/domain/sub-rooms';
import { unseenSubRooms } from '@/features/sub-rooms/domain/sub-room-browse';

const SEEN_KEY = 'agent-room.sub-rooms.seen.';

/**
 * 这台设备上见过这间房下面的哪些子房间（`specs/redesign/sub-rooms.md`：“有你没见过的新子房间时，
 * ‘浏览子房间’那一行带一个点”）。头一回打开时名单上的都记成见过；“浏览子房间”开着（`viewing`）时
 * 名单上的都记成见过。回的是有几间没见过的。
 */
export function useUnseenSubRooms(list: SubRoomList | null, viewing: boolean): number {
  const key = list === null ? null : SEEN_KEY + list.parent.catalogId;
  const ids = list?.subRooms.map((room) => room.catalogId).join(',') ?? '';
  const [seen, setSeen] = useState<{
    readonly key: string;
    readonly ids: ReadonlySet<string>;
  } | null>(null);
  useEffect(() => {
    if (key === null) return;
    const stored = readSeen(key);
    if (stored !== null && !viewing) {
      setSeen({ key, ids: stored });
      return;
    }
    const current = new Set(ids === '' ? [] : ids.split(','));
    writeSeen(key, current);
    setSeen({ key, ids: current });
  }, [ids, key, viewing]);
  return list === null || seen?.key !== key ? 0 : unseenSubRooms(list, seen.ids);
}

function readSeen(key: string): ReadonlySet<string> | null {
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === null) return null;
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? new Set(parsed.filter((id): id is string => typeof id === 'string'))
      : null;
  } catch {
    return null;
  }
}

function writeSeen(key: string, ids: ReadonlySet<string>): void {
  try {
    window.localStorage.setItem(key, JSON.stringify([...ids]));
  } catch {
    // 存不下就只在这一次打开里记着。
  }
}
