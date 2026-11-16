import { Pin, PinOff } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { RoomPinsChange } from '@/features/messages/domain/message';

import '@/features/room-notice/ui/room-notice-change.css';

export type RoomPinsChangeLineProps = {
  readonly change: RoomPinsChange;
  /** Matrix ID 到名字；认不出的写“房主或管理员”。 */
  readonly names: ReadonlyMap<string, string>;
  readonly selfMatrixUserId: string | null;
  readonly time: Intl.DateTimeFormat;
  /** 点“看那一句”跳过去；不给就只是一行字。 */
  readonly onJump?: ((eventId: string) => void) | undefined;
};

/**
 * 对话里按时间夹着的一行：“某某置顶了一句”“某某取消了一条置顶”（`specs/redesign/brief-and-pins.md`
 * “记录和未读”）。这几行就是置顶的记录，服务器上不另存；点“看那一句”跳到对话里那一句，没加载到就往前补。
 */
export function RoomPinsChangeLine({
  change,
  names,
  selfMatrixUserId,
  time,
  onJump,
}: RoomPinsChangeLineProps) {
  const { t } = useTranslation();
  const pinned = change.kind === 'pinned';
  const self = change.updatedBy === selfMatrixUserId;
  const name = self ? undefined : names.get(change.updatedBy);
  const text = self
    ? t(pinned ? 'roomPins.change.pinnedBySelf' : 'roomPins.change.unpinnedBySelf')
    : name === undefined
      ? t(pinned ? 'roomPins.change.pinnedBySomeone' : 'roomPins.change.unpinnedBySomeone')
      : t(pinned ? 'roomPins.change.pinned' : 'roomPins.change.unpinned', { name });
  return (
    <div className="room-notice-change">
      <p className="room-notice-change__line">
        {pinned ? <Pin aria-hidden="true" /> : <PinOff aria-hidden="true" />}
        <span>{text}</span>
        <time dateTime={new Date(change.serverTimestamp).toISOString()}>
          {time.format(change.serverTimestamp)}
        </time>
        {onJump === undefined ? null : (
          <button
            className="room-notice-change__toggle"
            onClick={() => {
              onJump(change.targetEventId);
            }}
            type="button"
          >
            {t('roomPins.change.show')}
          </button>
        )}
      </p>
    </div>
  );
}
