import { useState } from 'react';

import { usePersonalWorkspace } from '@/features/personal-workspace/ui/personal-workspace-provider';
import { pinsSeenKey } from '@/features/room-pins/domain/room-pins';

export type PinsSeen = {
  /** 看过的那一次置顶改的时间；没看过是 null。 */
  readonly seenAt: number | null;
  /** 个人工作区还在从服务器读的时候不知道看没看过：这时不亮点，免得换了设备的人看到点亮一下又灭。 */
  readonly known: boolean;
  /** 点开置顶的列表时记下看过这一次。 */
  readonly markSeen: (updatedAtUnixMs: number) => void;
};

/**
 * 看过哪一次置顶（`specs/redesign/brief-and-pins.md`“记录和未读”）：和须知一样记在个人工作区的
 * `acknowledged` 里，键是 `pins:<房间>`、值是那一次改的时间，换设备也算；合并时大的赢。工作区写不进去
 * （没登录、夹具）时只在这一页上记着。
 */
export function usePinsSeen(roomId: string): PinsSeen {
  const personal = usePersonalWorkspace();
  const key = pinsSeenKey(roomId);
  const signedIn = personal !== null && personal.snapshot.accountId !== null;
  const known = personal === null || (signedIn && personal.snapshot.status !== 'loading');
  const saved = key === null ? undefined : personal?.snapshot.index.acknowledgedAt.get(key);
  const [local, setLocal] = useState<number | null>(null);

  const markSeen = (updatedAtUnixMs: number): void => {
    if (
      personal !== null &&
      signedIn &&
      key !== null &&
      personal.change({ kind: 'acknowledged', id: key, value: updatedAtUnixMs }).ok
    )
      return;
    setLocal((previous) => Math.max(previous ?? 0, updatedAtUnixMs));
  };

  return {
    seenAt: saved === undefined ? local : local === null ? saved : Math.max(saved, local),
    known,
    markSeen,
  };
}
