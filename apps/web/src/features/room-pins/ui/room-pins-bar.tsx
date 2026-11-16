import { ChevronDown, ChevronUp, Pin } from 'lucide-react';
import { useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { ActorAvatar } from '@/features/lobby/ui/avatars';
import type { RoomMessageSignal } from '@/features/messages/domain/message';
import { useRoomMessages } from '@/features/messages/ui/room-messages-context';
import type { RoomPin, RoomPins } from '@/features/room-pins/domain/room-pins';
import { formatDateTime } from '@/shared/i18n/formatters';

import { usePinsSeen } from './use-pins-seen';

import './room-pins-bar.css';

export type RoomPinsBarProps = {
  readonly pins: RoomPins;
  /** Matrix 房间 ID：读这间房加载了的消息，看过哪一次按它记在个人工作区。换了房间要换一个组件（`key`）。 */
  readonly roomId: string;
  /** 这台设备上登录的人；自己改的不算没看过。 */
  readonly selfMatrixUserId: string | null;
  /** 点一条跳到对话里那一句（按 Matrix 事件 ID；没加载到就往前补）。 */
  readonly onJump: (eventId: string) => void;
};

const emptyMessages: readonly RoomMessageSignal[] = [];

/**
 * 须知下面那一行“置顶 N 条”（`specs/redesign/brief-and-pins.md`“置顶”）：平时一行，写最新那一条是谁说的、
 * 开头；点开列出每一句，点一句跳到对话里那一句。那一句写了什么、谁说的，照这台设备上已经加载、解开的
 * 那一条：服务器上只记事件 ID。改了以后你还没点开看过的，亮一个点；看没看过记在个人工作区里。
 */
export function RoomPinsBar({ pins, roomId, selfMatrixUserId, onJump }: RoomPinsBarProps) {
  const { t, i18n } = useTranslation();
  const listId = useId();
  const [expanded, setExpanded] = useState(false);
  const { seenAt, known, markSeen } = usePinsSeen(roomId);
  const { state } = useRoomMessages(roomId);
  const loaded = state.kind === 'ready' ? state.room.messages : emptyMessages;
  // 只认对话里的那几句（还有删了的）：资料卡片不在对话里，点了也跳不过去。
  const byEvent = useMemo(
    () =>
      new Map(
        loaded
          .filter(
            (message) =>
              message.lifecycle !== 'active' || message.preview?.conversation !== undefined,
          )
          .map((message) => [message.matrixEventId, message]),
      ),
    [loaded],
  );
  const unseen =
    known &&
    pins.updatedBy !== selfMatrixUserId &&
    (seenAt === null || seenAt < pins.updatedAtUnixMs);
  const toggle = (): void => {
    if (!expanded) markSeen(pins.updatedAtUnixMs);
    setExpanded(!expanded);
  };
  const [first] = pins.entries;
  const firstMessage = first === undefined ? undefined : byEvent.get(first.eventId);
  const summary =
    firstMessage?.lifecycle === 'active'
      ? t('roomPins.summary', { name: firstMessage.actor.displayName, text: textOf(firstMessage) })
      : null;

  return (
    <section aria-label={t('roomPins.label')} className="room-pins">
      <button
        aria-controls={listId}
        aria-expanded={expanded}
        className="room-pins__toggle"
        onClick={toggle}
        type="button"
      >
        <Pin aria-hidden="true" />
        <strong>{t('roomPins.title')}</strong>
        <span className="room-pins__count">{pins.entries.length}</span>
        {unseen ? (
          <span className="room-pins__dot">
            <span className="sr-only">{t('roomPins.unseen')}</span>
          </span>
        ) : null}
        {expanded || summary === null ? null : (
          <span className="room-pins__summary">{summary}</span>
        )}
        <span className="sr-only">{t(expanded ? 'roomPins.collapse' : 'roomPins.expand')}</span>
        {expanded ? <ChevronUp aria-hidden="true" /> : <ChevronDown aria-hidden="true" />}
      </button>
      <div hidden={!expanded} id={listId}>
        <ol className="room-pins__list">
          {pins.entries.map((pin) => (
            <li key={pin.eventId}>
              <PinnedItem
                language={i18n.language}
                message={byEvent.get(pin.eventId)}
                onJump={onJump}
                pin={pin}
              />
            </li>
          ))}
        </ol>
        <p className="room-pins__footer">{t('roomPins.footer')}</p>
      </div>
    </section>
  );
}

function PinnedItem({
  pin,
  message,
  language,
  onJump,
}: {
  readonly pin: RoomPin;
  readonly message: RoomMessageSignal | undefined;
  readonly language: string;
  readonly onJump: (eventId: string) => void;
}) {
  const { t } = useTranslation();
  if (message !== undefined && message.lifecycle !== 'active')
    return <p className="room-pins__item room-pins__item--gone">{t('roomPins.deleted')}</p>;
  return (
    <button
      className="room-pins__item"
      onClick={() => {
        onJump(pin.eventId);
      }}
      type="button"
    >
      {message === undefined ? (
        <span className="room-pins__pending">{t('roomPins.notLoaded')}</span>
      ) : (
        <>
          <span className="room-pins__avatar" aria-hidden="true">
            <ActorAvatar actor={message.actor} />
          </span>
          <span className="room-pins__meta">
            <strong>{message.actor.displayName}</strong>
            <time dateTime={new Date(message.serverTimestamp).toISOString()}>
              {formatDateTime(message.serverTimestamp, language)}
            </time>
          </span>
          <span className="room-pins__text">{textOf(message)}</span>
        </>
      )}
    </button>
  );
}

function textOf(message: RoomMessageSignal): string {
  return message.preview?.conversation?.text ?? '';
}
