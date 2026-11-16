import { Crown, Pin } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type {
  RoomNoticeChange,
  RoomOwnerHandover,
  RoomStateLine,
} from '@/features/messages/domain/message';

import './room-notice-change.css';

export type RoomNoticeChangeLineProps = {
  readonly change: RoomStateLine;
  /** Matrix ID 到名字；认不出的写“房主或管理员”“一位管理员”。 */
  readonly names: ReadonlyMap<string, string>;
  readonly selfMatrixUserId: string | null;
  readonly time: Intl.DateTimeFormat;
};

/**
 * 对话里按时间夹着的一行：“某某更新了公告”“某某撤掉了公告”，或者房主删除了账户、“房间交给了某某”。
 * 公告本身在对话上方那一条里看。
 */
export function RoomNoticeChangeLine({
  change,
  names,
  selfMatrixUserId,
  time,
}: RoomNoticeChangeLineProps) {
  const { t } = useTranslation();
  const handover = change.kind === 'owner_handover';
  const { key, name } = handover
    ? ownerHandoverLabel(change, names, selfMatrixUserId)
    : noticeChangeLabel(change, names, selfMatrixUserId);
  const text = name === undefined ? t(key) : t(key, { name });
  return (
    <p className="room-notice-change">
      {handover ? <Crown aria-hidden="true" /> : <Pin aria-hidden="true" />}
      <span>{text}</span>
      <time dateTime={new Date(change.serverTimestamp).toISOString()}>
        {time.format(change.serverTimestamp)}
      </time>
    </p>
  );
}

type LineLabel = {
  readonly key:
    | 'roomNotice.change.updated'
    | 'roomNotice.change.removed'
    | 'roomNotice.change.updatedBySelf'
    | 'roomNotice.change.removedBySelf'
    | 'roomNotice.change.updatedBySomeone'
    | 'roomNotice.change.removedBySomeone'
    | 'roomNotice.change.ownerHandover'
    | 'roomNotice.change.ownerHandoverToSelf'
    | 'roomNotice.change.ownerHandoverToSomeone';
  /** 认得出的人的名字；写“你”、写“房主或管理员”的不带。 */
  readonly name?: string;
};

function noticeChangeLabel(
  change: RoomNoticeChange,
  names: ReadonlyMap<string, string>,
  selfMatrixUserId: string | null,
): LineLabel {
  const updated = change.kind === 'updated';
  if (change.updatedBy === selfMatrixUserId)
    return {
      key: updated ? 'roomNotice.change.updatedBySelf' : 'roomNotice.change.removedBySelf',
    };
  const name = names.get(change.updatedBy);
  if (name === undefined)
    return {
      key: updated ? 'roomNotice.change.updatedBySomeone' : 'roomNotice.change.removedBySomeone',
    };
  return { key: updated ? 'roomNotice.change.updated' : 'roomNotice.change.removed', name };
}

function ownerHandoverLabel(
  handover: RoomOwnerHandover,
  names: ReadonlyMap<string, string>,
  selfMatrixUserId: string | null,
): LineLabel {
  if (handover.owner === selfMatrixUserId) return { key: 'roomNotice.change.ownerHandoverToSelf' };
  const name = names.get(handover.owner);
  return name === undefined
    ? { key: 'roomNotice.change.ownerHandoverToSomeone' }
    : { key: 'roomNotice.change.ownerHandover', name };
}
