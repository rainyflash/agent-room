import { useEffect, useState } from 'react';

import type { LobbyGateway, LobbyMemberName } from '@/features/lobby/domain/lobby';
import type { RoomNotice } from '@/features/room-notice/domain/room-notice';

/** 一间房此刻的须知，连同改的人的昵称（认不出就没有）。 */
export type RoomBrief = {
  readonly notice: RoomNotice;
  readonly author?: LobbyMemberName;
};

/**
 * 读另一间房的须知并跟着它变：子房间里先写父房间的须知（`specs/redesign/brief-and-pins.md`“子房间里
 * 父房间的须知也算”）。子房间只收父房间里的人，所以网页直接读父房间在 Matrix 上的状态。读不到（还没
 * 同步到、不在那间房里）时当没有。只在须知或改的人的昵称变了时换一份，父房间别的动静不让这里重画。
 */
export function useRoomBrief(lobby: LobbyGateway, roomId: string | null): RoomBrief | null {
  const [brief, setBrief] = useState<RoomBrief | null>(null);
  useEffect(() => {
    if (roomId === null) {
      setBrief(null);
      return undefined;
    }
    const refresh = (): void => {
      const next = readBrief(lobby, roomId);
      setBrief((previous) => (sameBrief(previous, next) ? previous : next));
    };
    refresh();
    return lobby.subscribe(roomId, refresh);
  }, [lobby, roomId]);
  return roomId === null ? null : brief;
}

function readBrief(lobby: LobbyGateway, roomId: string): RoomBrief | null {
  const read = lobby.read(roomId);
  if (!read.ok || read.value.notice === undefined) return null;
  const { notice, memberNames } = read.value;
  const author = memberNames?.find((member) => member.matrixUserId === notice.updatedBy);
  return Object.freeze({ notice, ...(author === undefined ? {} : { author }) });
}

function sameBrief(left: RoomBrief | null, right: RoomBrief | null): boolean {
  if (left === null || right === null) return left === right;
  return (
    left.notice.text === right.notice.text &&
    left.notice.updatedAtUnixMs === right.notice.updatedAtUnixMs &&
    left.notice.updatedBy === right.notice.updatedBy &&
    left.author?.displayName === right.author?.displayName
  );
}
