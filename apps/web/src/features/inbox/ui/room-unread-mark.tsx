import { useTranslation } from 'react-i18next';

import type { RoomUnread } from '../domain/inbox';
import { useInbox } from './inbox-provider';
import './room-unread-mark.css';

/** 超过这么多只写“99+”。 */
const COUNT_LIMIT = 99;

/** 这个私人房间有多少没读、有没有人点名你；收件箱还没读出来时是 undefined。 */
export function useRoomUnread(roomId: string): RoomUnread | undefined {
  return useInbox()?.snapshot.rooms.get(roomId);
}

/** 这个公开大厅的哪个分片里有人点名你。 */
export function useCatalogMentioned(catalogId: string): boolean {
  const rooms = useInbox()?.snapshot.rooms;
  if (rooms === undefined) return false;
  for (const unread of rooms.values())
    if (unread.catalogId === catalogId && unread.mentioned) return true;
  return false;
}

/**
 * “房间”页卡片上的未读：有人点名你或 @所有人时标 @，私人房间再写没读的条数
 * （`specs/workspace-tidy/design.md`）。公开大厅只标 @：人多，条数很快就是 99+。
 */
export function RoomUnreadMark({
  count,
  mentioned,
  more = false,
}: {
  readonly count?: number | null | undefined;
  readonly mentioned: boolean;
  readonly more?: boolean;
}) {
  const { t } = useTranslation();
  const shown = count !== null && count !== undefined && count > 0;
  if (!shown && !mentioned) return null;
  const label = shown
    ? count > COUNT_LIMIT
      ? `${String(COUNT_LIMIT)}+`
      : `${String(count)}${more ? '+' : ''}`
    : '';
  const spoken = [
    mentioned ? t('inbox.room.mentioned') : null,
    shown
      ? t(count > COUNT_LIMIT || more ? 'inbox.room.unreadAtLeast' : 'inbox.room.unread', {
          count: Math.min(count, COUNT_LIMIT),
        })
      : null,
  ]
    .filter((part) => part !== null)
    .join(t('inbox.room.separator'));
  return (
    <span className="room-unread-mark">
      <span className="sr-only">{spoken}</span>
      {mentioned ? (
        <span aria-hidden="true" className="room-unread-mark__mention">
          @
        </span>
      ) : null}
      {shown ? (
        <span aria-hidden="true" className="room-unread-mark__count">
          {label}
        </span>
      ) : null}
    </span>
  );
}
