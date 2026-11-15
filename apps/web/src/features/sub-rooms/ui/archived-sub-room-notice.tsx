import { Button } from '@agent-room/ui-system';
import { Archive, LoaderCircle, RotateCcw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { SubRoom, SubRoomList } from '@/features/private-rooms/domain/sub-rooms';

import { FailureNote } from './sub-room-browser';
import { useSubRoomActions } from './use-sub-room-actions';
import './sub-room-door.css';

export type ArchivedSubRoomNoticeProps = {
  /** 这间子房间所在的整间房下面归档了的那一份：能不能恢复看它。 */
  readonly list: SubRoomList;
  /** 归档名单里的这一间；还没问回来时为空，先不给“恢复”。 */
  readonly room: SubRoom | undefined;
};

/**
 * 归档了的子房间（`specs/redesign/sub-rooms.md`“网页”）：对话照常能看，对话上面写“这间子房间已经归档”，
 * 能恢复的人旁边有“恢复”。恢复以后这一行收起来，原来的人接着说。
 */
export function ArchivedSubRoomNotice({ list, room }: ArchivedSubRoomNoticeProps) {
  const { t } = useTranslation();
  const actions = useSubRoomActions(list.parent.catalogId);
  const busy = room !== undefined && actions.busy?.catalogId === room.catalogId;
  return (
    <div className="room-panel__read-only sub-room-archived" role="status">
      <Archive aria-hidden="true" />
      <span className="sub-room-archived__text">{t('subRooms.door.archived')}</span>
      {room?.canRestore === true ? (
        <Button
          aria-busy={busy}
          disabled={busy}
          icon={
            busy ? (
              <LoaderCircle aria-hidden="true" className="sub-room-browser__spin" />
            ) : (
              <RotateCcw aria-hidden="true" />
            )
          }
          onClick={() => {
            actions.restore(room);
          }}
          size="compact"
          tone="ghost"
        >
          {t('subRooms.browse.action.restore')}
        </Button>
      ) : null}
      {room === undefined ? null : <FailureNote actions={actions} room={room} />}
    </div>
  );
}
