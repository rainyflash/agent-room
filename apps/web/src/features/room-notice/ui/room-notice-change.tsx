import { Pin } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { RoomNoticeChange } from '@/features/messages/domain/message';

import './room-notice-change.css';

export type RoomNoticeChangeLineProps = {
  readonly change: RoomNoticeChange;
  /** Matrix ID 到名字；认不出的写“房主或管理员”。 */
  readonly names: ReadonlyMap<string, string>;
  readonly selfMatrixUserId: string | null;
  readonly time: Intl.DateTimeFormat;
};

/** 对话里按时间夹着的一行“某某更新了公告”“某某撤掉了公告”。公告本身在对话上方那一条里看。 */
export function RoomNoticeChangeLine({
  change,
  names,
  selfMatrixUserId,
  time,
}: RoomNoticeChangeLineProps) {
  const { t } = useTranslation();
  const name = names.get(change.updatedBy);
  const text =
    change.updatedBy === selfMatrixUserId
      ? t(
          change.kind === 'updated'
            ? 'roomNotice.change.updatedBySelf'
            : 'roomNotice.change.removedBySelf',
        )
      : name === undefined
        ? t(
            change.kind === 'updated'
              ? 'roomNotice.change.updatedBySomeone'
              : 'roomNotice.change.removedBySomeone',
          )
        : t(change.kind === 'updated' ? 'roomNotice.change.updated' : 'roomNotice.change.removed', {
            name,
          });
  return (
    <p className="room-notice-change">
      <Pin aria-hidden="true" />
      <span>{text}</span>
      <time dateTime={new Date(change.serverTimestamp).toISOString()}>
        {time.format(change.serverTimestamp)}
      </time>
    </p>
  );
}
