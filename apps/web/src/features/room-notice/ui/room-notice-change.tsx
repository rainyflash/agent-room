import { Crown, Pin } from 'lucide-react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { RoomNoticeChange, RoomOwnerHandover } from '@/features/messages/domain/message';

import './room-notice-change.css';

export type RoomNoticeChangeLineProps = {
  /** 置顶改了的那一行另有 `RoomPinsChangeLine`。 */
  readonly change: RoomNoticeChange | RoomOwnerHandover;
  /** Matrix ID 到名字；认不出的写“房主或管理员”“一位管理员”。 */
  readonly names: ReadonlyMap<string, string>;
  readonly selfMatrixUserId: string | null;
  readonly time: Intl.DateTimeFormat;
};

/**
 * 对话里按时间夹着的一行：“某某改了须知”“某某撤掉了须知”，或者房主删除了账户、“房间交给了某某”。
 * 改了须知的那一行点开看这一次改成了什么（`specs/redesign/brief-and-pins.md`“记录”）：正文就在那条状态
 * 事件里，往前那几行就是以前的版本，不另做历史页。当前的须知在对话上方那一条里看。
 */
export function RoomNoticeChangeLine({
  change,
  names,
  selfMatrixUserId,
  time,
}: RoomNoticeChangeLineProps) {
  const { t } = useTranslation();
  const versionId = useId();
  const [open, setOpen] = useState(false);
  const handover = change.kind === 'owner_handover';
  const { key, name } = handover
    ? ownerHandoverLabel(change, names, selfMatrixUserId)
    : noticeChangeLabel(change, names, selfMatrixUserId);
  const text = name === undefined ? t(key) : t(key, { name });
  const version = change.kind === 'updated' && change.text !== '' ? change.text : null;
  return (
    <div className="room-notice-change">
      <p className="room-notice-change__line">
        {handover ? <Crown aria-hidden="true" /> : <Pin aria-hidden="true" />}
        <span>{text}</span>
        <time dateTime={new Date(change.serverTimestamp).toISOString()}>
          {time.format(change.serverTimestamp)}
        </time>
        {version === null ? null : (
          <button
            aria-controls={versionId}
            aria-expanded={open}
            className="room-notice-change__toggle"
            onClick={() => {
              setOpen(!open);
            }}
            type="button"
          >
            {t(open ? 'roomNotice.change.hideVersion' : 'roomNotice.change.showVersion')}
          </button>
        )}
      </p>
      {version === null ? null : (
        <div className="room-notice-change__version" hidden={!open} id={versionId}>
          <p className="room-notice-change__version-label">{t('roomNotice.change.versionLabel')}</p>
          <p className="room-notice-change__version-text">{version}</p>
        </div>
      )}
    </div>
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
