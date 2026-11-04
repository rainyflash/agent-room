import { Button } from '@agent-room/ui-system';
import {
  Bot,
  CalendarDays,
  CircleAlert,
  Clock3,
  Lock,
  LockOpen,
  LoaderCircle,
  Radio,
  Share2,
  UsersRound,
  X,
} from 'lucide-react';
import { useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import {
  avatarLetter,
  avatarTone,
  type ExploreFailure,
  type ExploreFilter,
  type ExploreRoomDetail,
} from '@/features/explore/domain/explore';
import { ShareRoomDialog } from '@/features/room-share/ui/share-room-dialog';
import { formatRelativeTime } from '@/shared/i18n/formatters';

import { ExploreRoomActions } from './explore-room-actions';
import type { ExploreAccount } from './explore-view';
import type { ExploreActions } from './use-explore-actions';

export type ExploreRoomDetailState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'failed'; readonly failure: ExploreFailure }
  | { readonly kind: 'ready'; readonly room: ExploreRoomDetail };

export type ExploreRoomPanelProps = {
  readonly account: ExploreAccount;
  readonly actions?: ExploreActions | undefined;
  readonly detail: ExploreRoomDetailState;
  readonly filter: ExploreFilter;
  readonly now: number;
  readonly onClose: () => void;
  /** 登录回来要申请的：申请框一打开就把光标放进去。 */
  readonly focusRequest?: boolean;
};

/**
 * 房间详情：点卡片从右边滑出来（手机上铺满）。头像、名字、房主和管理员、开了多久、整段简介、人数、
 * 现在几个在线、加入方式和加不加密；要申请的就在这里留一句话申请。
 */
export function ExploreRoomPanel({
  account,
  actions,
  detail,
  filter,
  focusRequest = false,
  now,
  onClose,
}: ExploreRoomPanelProps) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useLayoutEffect(() => {
    const element = dialog.current;
    if (element === null) return;
    const trigger = document.activeElement;
    if (typeof element.showModal === 'function') element.showModal();
    else element.setAttribute('open', '');
    return () => {
      if (typeof element.close === 'function') element.close();
      else element.removeAttribute('open');
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus();
    };
  }, []);
  return (
    <dialog
      aria-labelledby={detail.kind === 'ready' ? titleId : undefined}
      aria-label={detail.kind === 'ready' ? undefined : t('explore.room.loading')}
      className="explore-panel"
      onCancel={(event) => {
        // 叠在上面的分享对话框按 Esc 时，React 把它的 cancel 顺着组件树也传到这里：只管自己的。
        if (event.target !== event.currentTarget) return;
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        // 点在面板外的暗处就关上。
        if (event.target === event.currentTarget) onClose();
      }}
      ref={dialog}
    >
      <div className="explore-panel__body">
        <button
          aria-label={t('explore.room.close')}
          className="ar-icon-button explore-panel__close"
          onClick={onClose}
          type="button"
        >
          <X aria-hidden="true" />
        </button>
        {detail.kind === 'loading' ? (
          <p className="explore-panel__status" role="status">
            <LoaderCircle aria-hidden="true" className="explore__spin" />
            {t('explore.room.loading')}
          </p>
        ) : detail.kind === 'failed' ? (
          <PanelFailure failure={detail.failure} />
        ) : (
          <RoomDetail
            account={account}
            actions={actions}
            filter={filter}
            focusRequest={focusRequest}
            now={now}
            room={detail.room}
            titleId={titleId}
          />
        )}
      </div>
    </dialog>
  );
}

function PanelFailure({ failure }: { readonly failure: ExploreFailure }) {
  const { t } = useTranslation();
  const gone = failure.code === 'public_room.not_found';
  const limited = failure.code === 'public_rooms.rate_limited';
  return (
    <div className="explore-panel__status" role="alert">
      <CircleAlert aria-hidden="true" />
      <div>
        <h2>{t(gone ? 'explore.room.notFound.title' : 'explore.room.failed.title')}</h2>
        <p>
          {gone
            ? t('explore.room.notFound.detail')
            : limited
              ? t('explore.rateLimited')
              : t('explore.failed.detail')}
        </p>
        {gone || limited ? null : <code>{failure.code}</code>}
      </div>
    </div>
  );
}

function RoomDetail({
  account,
  actions,
  filter,
  focusRequest,
  now,
  room,
  titleId,
}: {
  readonly account: ExploreAccount;
  readonly actions?: ExploreActions | undefined;
  readonly filter: ExploreFilter;
  readonly focusRequest: boolean;
  readonly now: number;
  readonly room: ExploreRoomDetail;
  readonly titleId: string;
}) {
  const { i18n, t } = useTranslation();
  const language = i18n.resolvedLanguage;
  const listFormat = new Intl.ListFormat(language, { style: 'narrow', type: 'conjunction' });
  const [sharing, setSharing] = useState(false);
  return (
    <article className="explore-panel__room">
      <header className="explore-panel__header">
        <span
          aria-hidden="true"
          className={`explore-avatar explore-avatar--large explore-avatar--${avatarTone(room.catalogId)}`}
        >
          {avatarLetter(room.name)}
        </span>
        <div>
          <h2 id={titleId}>{room.name}</h2>
          {room.ownerName === '' ? null : (
            <p>{t('explore.card.owner', { name: room.ownerName })}</p>
          )}
          {room.admins.length === 0 ? null : (
            <p>{t('explore.room.admins', { names: listFormat.format(room.admins) })}</p>
          )}
          {/* 没进房间的人也能转给朋友；只能邀请的不给，链接打开了也进不来。 */}
          {room.joinMode === 'invite_only' ? null : (
            <Button
              aria-haspopup="dialog"
              aria-label={t('roomShare.openRoom', { name: room.name })}
              className="explore-panel__share"
              icon={<Share2 aria-hidden="true" />}
              onClick={() => {
                setSharing(true);
              }}
              size="compact"
              tone="ghost"
            >
              {t('roomShare.open')}
            </Button>
          )}
        </div>
      </header>
      {sharing ? (
        <ShareRoomDialog
          onClose={() => {
            setSharing(false);
          }}
          room={room}
        />
      ) : null}
      {room.description === '' ? null : (
        <p className="explore-panel__description">{room.description}</p>
      )}
      <ul className="explore-panel__facts">
        <PanelFact icon={<UsersRound aria-hidden="true" />}>
          {t('explore.card.people', { count: room.memberCount })}
        </PanelFact>
        <PanelFact icon={<Bot aria-hidden="true" />}>
          {t('explore.card.agents', { count: room.agentCount })}
        </PanelFact>
        {room.online === null ? null : (
          <PanelFact icon={<Radio aria-hidden="true" />}>
            {t(room.online.atLeast ? 'explore.room.onlineAtLeast' : 'explore.room.online', {
              count: room.online.count,
            })}
          </PanelFact>
        )}
        <PanelFact icon={<Clock3 aria-hidden="true" />}>
          {room.lastMessageAtUnixMs === null
            ? t('explore.card.quiet')
            : t('explore.card.lastSpoke', {
                time: formatRelativeTime(room.lastMessageAtUnixMs, now, language),
              })}
        </PanelFact>
        <PanelFact icon={<CalendarDays aria-hidden="true" />}>
          {t('explore.room.opened', {
            time: formatRelativeTime(room.createdAtUnixMs, now, language),
          })}
        </PanelFact>
      </ul>
      <dl className="explore-panel__terms">
        <div>
          <dt>{t(`privateRooms.listing.joinMode.${room.joinMode}.label`)}</dt>
          <dd>{t(`explore.room.joinMode.${room.joinMode}`)}</dd>
        </div>
        <div>
          <dt>
            {room.endToEndEncrypted ? <Lock aria-hidden="true" /> : <LockOpen aria-hidden="true" />}
            {t(room.endToEndEncrypted ? 'explore.card.encrypted' : 'explore.card.plain')}
          </dt>
          <dd>{t(room.endToEndEncrypted ? 'explore.room.encrypted' : 'explore.room.plain')}</dd>
        </div>
      </dl>
      <ViewerNote room={room} />
      <div className="explore-panel__actions">
        <ExploreRoomActions
          account={account}
          actions={actions}
          filter={filter}
          focusRequest={focusRequest}
          place="panel"
          room={room}
        />
      </div>
    </article>
  );
}

function ViewerNote({ room }: { readonly room: ExploreRoomDetail }) {
  const { t } = useTranslation();
  const note =
    room.viewer === 'pending'
      ? t('explore.room.pending')
      : room.viewer === 'invited'
        ? t('explore.room.invited')
        : room.viewer === 'banned'
          ? t('explore.room.banned')
          : null;
  return note === null ? null : <p className="explore-panel__viewer">{note}</p>;
}

function PanelFact({ children, icon }: { readonly children: ReactNode; readonly icon: ReactNode }) {
  return (
    <li>
      {icon}
      <span>{children}</span>
    </li>
  );
}
