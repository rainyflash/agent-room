import { ChevronDown, ChevronUp, Pencil, Pin } from 'lucide-react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { LobbyMemberName } from '@/features/lobby/domain/lobby';
import { markNoticeSeen, seenNoticeAt } from '@/features/room-notice/data/seen-notices';
import type { RoomNotice } from '@/features/room-notice/domain/room-notice';
import { formatDateTime } from '@/shared/i18n/formatters';

import './room-notice-bar.css';

export type RoomNoticeBarProps = {
  readonly notice: RoomNotice;
  /** Matrix 房间 ID：看过哪一版按它记在本机。换了房间要换一个组件（`key`）。 */
  readonly roomId: string;
  readonly memberNames?: readonly LobbyMemberName[] | undefined;
  /** 这台设备上登录的人；自己改的不算没看过。 */
  readonly selfMatrixUserId: string | null;
  /** 房主和管理员能改；不给就没有“编辑”。 */
  readonly onEdit?: (() => void) | undefined;
};

/**
 * 对话上方的“公告”一条（`specs/public-rooms/design.md`“常驻公告”）：平时只显示开头两行，点开看全文和
 * 谁什么时候改的。改了以后你还没点开看过的，亮一个点。
 */
export function RoomNoticeBar({
  notice,
  roomId,
  memberNames,
  selfMatrixUserId,
  onEdit,
}: RoomNoticeBarProps) {
  const { t, i18n } = useTranslation();
  const textId = useId();
  const [expanded, setExpanded] = useState(false);
  const [seenAt, setSeenAt] = useState(() => seenNoticeAt(roomId));
  const unseen =
    notice.updatedBy !== selfMatrixUserId && (seenAt === null || seenAt < notice.updatedAtUnixMs);
  const toggle = (): void => {
    if (!expanded) {
      markNoticeSeen(roomId, notice.updatedAtUnixMs);
      setSeenAt(notice.updatedAtUnixMs);
    }
    setExpanded(!expanded);
  };
  const author =
    notice.updatedBy === selfMatrixUserId
      ? t('roomNotice.you')
      : (memberNames?.find((member) => member.matrixUserId === notice.updatedBy)?.displayName ??
        t('roomNotice.someone'));

  return (
    <section aria-label={t('roomNotice.label')} className="room-notice">
      <div className="room-notice__head">
        <button
          aria-controls={textId}
          aria-expanded={expanded}
          className="room-notice__toggle"
          onClick={toggle}
          type="button"
        >
          <Pin aria-hidden="true" />
          <strong>{t('roomNotice.title')}</strong>
          {unseen ? (
            <span className="room-notice__dot">
              <span className="sr-only">{t('roomNotice.unseen')}</span>
            </span>
          ) : null}
          <span className="room-notice__toggle-label">
            {t(expanded ? 'roomNotice.collapse' : 'roomNotice.expand')}
          </span>
          {expanded ? <ChevronUp aria-hidden="true" /> : <ChevronDown aria-hidden="true" />}
        </button>
        {onEdit === undefined ? null : (
          <button
            aria-label={t('roomNotice.edit')}
            className="ar-icon-button"
            onClick={onEdit}
            title={t('roomNotice.edit')}
            type="button"
          >
            <Pencil aria-hidden="true" />
          </button>
        )}
      </div>
      <p
        className={expanded ? 'room-notice__text' : 'room-notice__text room-notice__text--clamped'}
        id={textId}
      >
        {notice.text}
      </p>
      {expanded ? (
        <p className="room-notice__byline">
          {t('roomNotice.byline', {
            name: author,
            time: formatDateTime(notice.updatedAtUnixMs, i18n.language),
          })}
        </p>
      ) : null}
    </section>
  );
}
