import { useAppServices } from '@/app/app-services';
import type { LobbyMemberName } from '@/features/lobby/domain/lobby';
import type { RoomNotice } from '@/features/room-notice/domain/room-notice';

import { RoomNoticeBar } from './room-notice-bar';
import { useRoomBrief } from './use-room-brief';

export type RoomBriefsProps = {
  /** 眼前这间房。 */
  readonly room: {
    readonly roomId: string;
    readonly notice?: RoomNotice | undefined;
    readonly memberNames?: readonly LobbyMemberName[] | undefined;
  };
  /** 眼前是子房间时它的父房间；父房间的 Matrix 房间号还没问到时是 null，先只写这一间的。 */
  readonly parent: { readonly matrixRoomId: string | null; readonly name: string } | null;
  readonly selfMatrixUserId: string | null;
  /** 能改这一间的须知时给；父房间那条在这里不给改，到父房间里改。 */
  readonly onEdit?: (() => void) | undefined;
};

/**
 * 对话上面的须知（`specs/redesign/brief-and-pins.md`）。子房间里先写父房间的须知、标着来自哪间房，再写这一间
 * 自己的；两段都没有就什么都不写。父房间那条看没看过按父房间记，在父房间里看过的这里也不亮。
 */
export function RoomBriefs({ room, parent, selfMatrixUserId, onEdit }: RoomBriefsProps) {
  const { lobby } = useAppServices();
  const parentRoomId = parent?.matrixRoomId ?? null;
  const parentBrief = useRoomBrief(lobby, parentRoomId);
  const fromParent = parent !== null && parentRoomId !== null ? parentBrief : null;
  return (
    <>
      {fromParent === null || parent === null || parentRoomId === null ? null : (
        <RoomNoticeBar
          key={`parent:${parentRoomId}`}
          memberNames={fromParent.author === undefined ? undefined : [fromParent.author]}
          notice={fromParent.notice}
          part={{ kind: 'parent', name: parent.name }}
          roomId={parentRoomId}
          selfMatrixUserId={selfMatrixUserId}
        />
      )}
      {room.notice === undefined ? null : (
        <RoomNoticeBar
          key={room.roomId}
          memberNames={room.memberNames}
          notice={room.notice}
          onEdit={onEdit}
          part={fromParent === null ? undefined : { kind: 'own' }}
          roomId={room.roomId}
          selfMatrixUserId={selfMatrixUserId}
        />
      )}
    </>
  );
}
