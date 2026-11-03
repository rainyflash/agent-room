import { Button } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { ArrowRight, LoaderCircle } from 'lucide-react';
import { useId, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';

import {
  exploreSearchFromFilter,
  type ExploreFilter,
  type ExploreRoom,
} from '@/features/explore/domain/explore';
import { JOIN_REQUEST_MESSAGE_MAX_LENGTH } from '@/features/private-rooms/domain/public-room-join';

import type { ExploreAccount } from './explore-view';
import type { ExploreActionKind, ExploreActions } from './use-explore-actions';

/** 说得清的几种没办成；别的说“过一会儿再试”，错误码收在后面。 */
const failureMessages = {
  'room_join.approval_required': 'explore.failure.room_join.approval_required',
  'room_join.banned': 'explore.failure.room_join.banned',
  'room_join.conflict': 'explore.failure.room_join.conflict',
  'room_join.invalid': 'explore.failure.room_join.invalid',
  'room_join.invite_only': 'explore.failure.room_join.invite_only',
  'room_join.not_found': 'explore.failure.room_join.not_found',
  'room_join.recently_rejected': 'explore.failure.room_join.recently_rejected',
} as const;

function knownFailure(code: string): code is keyof typeof failureMessages {
  return Object.hasOwn(failureMessages, code);
}

export function exploreFailureMessage(t: TFunction, code: string): string {
  return knownFailure(code) ? t(failureMessages[code]) : t('explore.failure.other');
}

/**
 * 一间公开房间的按钮，按你的处境写：已经在里面的直接进；没登录的先登录；收到过邀请的接受；直接进的
 * 一点就进；在等批准的能撤回；要申请的在卡片上打开详情，在详情里留一句话申请。被封禁、只能邀请的
 * 不给按钮。
 */
export function ExploreRoomActions({
  account,
  actions,
  filter,
  place,
  room,
}: {
  readonly account: ExploreAccount;
  readonly actions?: ExploreActions | undefined;
  readonly filter: ExploreFilter;
  readonly place: 'card' | 'panel';
  readonly room: ExploreRoom;
}) {
  const { t } = useTranslation();
  const name = room.name;
  if (room.viewer === 'joined') {
    return (
      <Link
        aria-label={`${t('explore.action.enter')} · ${name}`}
        className="ar-button ar-button--compact ar-button--primary"
        params={{ catalogId: room.catalogId }}
        search={{}}
        to="/lobby/$catalogId"
      >
        {t('explore.action.enter')}
        <ArrowRight aria-hidden="true" />
      </Link>
    );
  }
  if (room.viewer === 'banned') return null;
  if (room.joinMode === 'invite_only' && room.viewer !== 'invited') return null;
  if (account.kind === 'signed-out') {
    return (
      <Button
        aria-label={`${t('explore.action.signIn')} · ${name}`}
        onClick={account.onSignIn}
        size="compact"
        tone="ghost"
      >
        {t('explore.action.signIn')}
      </Button>
    );
  }
  if (actions === undefined) return null;
  const busy = actions.busy?.catalogId === room.catalogId ? actions.busy.kind : null;
  const failure = actions.failure?.catalogId === room.catalogId ? actions.failure.code : null;
  const failureNote =
    failure === null ? null : (
      <p className="explore-actions__failure" role="alert">
        {exploreFailureMessage(t, failure)}
        {knownFailure(failure) ? null : <code>{failure}</code>}
      </p>
    );
  if (room.viewer === 'pending') {
    return (
      <div className="explore-actions">
        <span className="explore-actions__state">{t('explore.action.pending')}</span>
        <ActionButton
          busy={busy === 'withdraw'}
          disabled={busy !== null}
          label={t('explore.action.withdrawRoom', { name })}
          onClick={() => {
            actions.onWithdraw(room);
          }}
          tone="ghost"
        >
          {t('explore.action.withdraw')}
        </ActionButton>
        {failureNote}
      </div>
    );
  }
  if (room.viewer !== 'invited' && room.joinMode === 'request') {
    return place === 'panel' ? (
      <RequestForm actions={actions} busy={busy} failureNote={failureNote} room={room} />
    ) : (
      <Link
        aria-label={`${t('explore.action.request')} · ${name}`}
        className="ar-button ar-button--compact ar-button--primary"
        params={{ code: room.shareCode }}
        search={exploreSearchFromFilter(filter)}
        to="/explore/$code"
      >
        {t('explore.action.request')}
      </Link>
    );
  }
  const action = room.viewer === 'invited' ? 'invited' : 'join';
  return (
    <div className="explore-actions">
      <ActionButton
        busy={busy === 'join'}
        disabled={busy !== null}
        label={`${t(`explore.action.${action}`)} · ${name}`}
        onClick={() => {
          actions.onJoin(room);
        }}
      >
        {t(`explore.action.${action}`)}
      </ActionButton>
      {failureNote}
    </div>
  );
}

function ActionButton({
  busy,
  children,
  disabled,
  label,
  onClick,
  tone = 'primary',
}: {
  readonly busy: boolean;
  readonly children: string;
  readonly disabled: boolean;
  readonly label: string;
  readonly onClick: () => void;
  readonly tone?: 'primary' | 'ghost';
}) {
  return (
    <Button
      aria-busy={busy}
      aria-label={label}
      disabled={disabled}
      icon={busy ? <LoaderCircle aria-hidden="true" className="explore__spin" /> : undefined}
      onClick={onClick}
      size="compact"
      tone={tone}
    >
      {children}
    </Button>
  );
}

/** 申请：留一句话（可以不写，最多 200 字），就在详情里，不用再跳一页。 */
function RequestForm({
  actions,
  busy,
  failureNote,
  room,
}: {
  readonly actions: ExploreActions;
  readonly busy: ExploreActionKind | null;
  readonly failureNote: ReactNode;
  readonly room: ExploreRoom;
}) {
  const { i18n, t } = useTranslation();
  const messageId = useId();
  const countId = useId();
  const [message, setMessage] = useState('');
  // 和服务器一样按字算：表情、汉字都算一个。
  const length = Array.from(message).length;
  const tooLong = length > JOIN_REQUEST_MESSAGE_MAX_LENGTH;
  return (
    <form
      className="explore-request"
      onSubmit={(event) => {
        event.preventDefault();
        if (!tooLong && busy === null) actions.onRequest(room, message);
      }}
    >
      <label htmlFor={messageId}>{t('explore.room.message')}</label>
      <textarea
        aria-describedby={countId}
        aria-invalid={tooLong}
        disabled={busy !== null}
        id={messageId}
        onChange={(event) => {
          setMessage(event.target.value);
        }}
        rows={3}
        value={message}
      />
      <div className="explore-request__footer">
        <small className={tooLong ? 'explore-request__count--over' : undefined} id={countId}>
          {t('explore.room.messageCount', {
            count: length,
            max: JOIN_REQUEST_MESSAGE_MAX_LENGTH,
            lng: i18n.resolvedLanguage,
          })}
        </small>
        <Button
          aria-busy={busy === 'request'}
          disabled={busy !== null || tooLong}
          icon={
            busy === 'request' ? (
              <LoaderCircle aria-hidden="true" className="explore__spin" />
            ) : undefined
          }
          size="compact"
          type="submit"
        >
          {t('explore.action.request')}
        </Button>
      </div>
      {failureNote}
    </form>
  );
}
