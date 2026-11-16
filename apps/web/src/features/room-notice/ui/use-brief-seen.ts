import { useEffect, useRef, useState } from 'react';

import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import {
  forgetNoticeSeen,
  markNoticeSeen,
  seenNoticeAt,
} from '@/features/room-notice/data/seen-notices';
import { briefSeenKey } from '@/features/room-notice/domain/room-notice';

export type BriefSeen = {
  /** 看过的那一版须知改的时间；没看过是 null。 */
  readonly seenAt: number | null;
  /**
   * 知不知道看没看过。个人工作区还在从服务器读的时候不知道：这时亮点，换了设备的人会先看到点亮一下又灭。
   * 没有个人工作区的页面（夹具）只看这个浏览器记着的。
   */
  readonly known: boolean;
  /** 点开须知时记下看过这一版。 */
  readonly markSeen: (updatedAtUnixMs: number) => void;
};

/**
 * 看过哪一版须知（`specs/redesign/brief-and-pins.md`“记录和未读”）：记在个人工作区的 `acknowledged` 里，
 * 键是 `brief:<房间>`、值是那一版改的时间，换设备也算；合并时大的赢，旧设备连回来不会把它往回改。
 * 以前只记在这个浏览器里的，第一次打开这间房时搬进去；工作区写不进去时先记在这个浏览器里。
 */
export function useBriefSeen(roomId: string): BriefSeen {
  const personal = usePersonalWorkspace();
  const key = briefSeenKey(roomId);
  const signedIn = personal !== null && personal.snapshot.accountId !== null;
  const known = personal === null || (signedIn && personal.snapshot.status !== 'loading');
  const saved = key === null ? undefined : personal?.snapshot.index.acknowledgedAt.get(key);
  const [local, setLocal] = useState(() => seenNoticeAt(roomId));
  const moved = useRef<number | null>(null);

  useEffect(() => {
    if (personal === null || !signedIn || key === null || local === null) return;
    if (moved.current === local) return;
    moved.current = local;
    if (personal.change({ kind: 'acknowledged', id: key, value: local }).ok)
      forgetNoticeSeen(roomId);
  }, [personal, signedIn, key, local, roomId]);

  const markSeen = (updatedAtUnixMs: number): void => {
    if (
      personal !== null &&
      signedIn &&
      key !== null &&
      personal.change({ kind: 'acknowledged', id: key, value: updatedAtUnixMs }).ok
    )
      return;
    markNoticeSeen(roomId, updatedAtUnixMs);
    setLocal((previous) => Math.max(previous ?? 0, updatedAtUnixMs));
  };

  return { seenAt: later(saved, local), known, markSeen };
}

function later(left: number | undefined, right: number | null): number | null {
  if (left === undefined) return right;
  return right === null ? left : Math.max(left, right);
}
