import { Button } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import {
  ArrowRight,
  Bot,
  CircleAlert,
  Clock3,
  Eye,
  LoaderCircle,
  Lock,
  LockOpen,
  Plus,
  RefreshCw,
  Search,
  UsersRound,
} from 'lucide-react';
import { useEffect, useId, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import {
  avatarLetter,
  avatarTone,
  EXPLORE_SEARCH_MAX_LENGTH,
  exploreQuickFilters,
  exploreSearchFromFilter,
  exploreSorts,
  type ExploreFailure,
  type ExploreFilter,
  type ExploreRoom,
} from '@/features/explore/domain/explore';
import { LanguageControl } from '@/features/preferences/ui/language-control';
import type { PublicRoomSummary } from '@/features/room-directory/domain/public-room-directory';
import { WebHeaderLinks } from '@/features/updates/ui/web-header-links';
import { formatRelativeTime } from '@/shared/i18n/formatters';
import { AppNavigation } from '@/shared/ui/app-navigation';

import { ExploreRoomActions } from './explore-room-actions';
import { ExploreRoomPanel, type ExploreRoomDetailState } from './explore-room-panel';
import type { ExploreActions } from './use-explore-actions';

import './explore.css';

/** 搜索框停手这么久才去问服务器，打一个字问一次太多（还按来源限流）。 */
const SEARCH_DEBOUNCE_MS = 300;

/** 还在问登录了没有、登录了、没登录（顶栏是“登录”“注册”，卡片上是“登录后加入”）。 */
export type ExploreAccount =
  | { readonly kind: 'checking' }
  | { readonly kind: 'signed-in' }
  | {
      readonly kind: 'signed-out';
      readonly registrationOpen: boolean;
      readonly onRegister: () => void;
      readonly onSignIn: () => void;
    };

export type ExploreRoomsState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'failed'; readonly failure: ExploreFailure }
  | {
      readonly kind: 'ready';
      readonly more: 'none' | 'available' | 'loading';
      readonly rooms: readonly ExploreRoom[];
    };

export type ExploreViewProps = {
  readonly account: ExploreAccount;
  /** 登录了才有：加入、申请、撤回。 */
  readonly actions?: ExploreActions | undefined;
  readonly filter: ExploreFilter;
  readonly lobbies: readonly PublicRoomSummary[];
  readonly now: number;
  /** 登录了才有：打开新建房间、一开始就选着公开。 */
  readonly onCreatePublicRoom?: (() => void) | undefined;
  readonly onFilterChange: (filter: ExploreFilter) => void;
  readonly onLoadMore: () => void;
  readonly onRetry: () => void;
  /** 打开着一间房的详情（`/explore/<分享码>`）。 */
  readonly panel?:
    { readonly detail: ExploreRoomDetailState; readonly onClose: () => void } | undefined;
  readonly rooms: ExploreRoomsState;
};

/**
 * 发现页（`specs/public-rooms/design.md`“发现页”）：搜索、快捷筛选、排序，最上面一排公共大厅，下面是
 * 公开房间的卡片。点卡片打开这间房的详情；卡片上的按钮按你的处境写。不登录也能看。
 */
export function ExploreView({
  account,
  actions,
  filter,
  lobbies,
  now,
  onCreatePublicRoom,
  onFilterChange,
  onLoadMore,
  onRetry,
  panel,
  rooms,
}: ExploreViewProps) {
  const { t } = useTranslation();
  const roomsHeadingId = useId();
  return (
    <main className="explore" id="main-content">
      {/* 和围观页一样不进搜索：房间随时可能改回私密。 */}
      <meta content="noindex" name="robots" />
      {account.kind === 'signed-out' ? (
        <ExploreGuestHeader account={account} />
      ) : (
        <AppNavigation active="explore" />
      )}
      <section className="explore__hero">
        <h1>{t('explore.title')}</h1>
        <p>{t('explore.description')}</p>
      </section>
      <ExploreFilters filter={filter} onFilterChange={onFilterChange} />
      {lobbies.length === 0 ? null : (
        <LobbyRow lobbies={lobbies} signedIn={account.kind === 'signed-in'} />
      )}
      <section aria-labelledby={roomsHeadingId} className="explore__section">
        <h2 className="explore__heading" id={roomsHeadingId}>
          {t('explore.rooms')}
        </h2>
        <RoomResults
          account={account}
          actions={actions}
          filter={filter}
          now={now}
          onCreatePublicRoom={onCreatePublicRoom}
          onLoadMore={onLoadMore}
          onRetry={onRetry}
          rooms={rooms}
        />
      </section>
      {panel === undefined ? null : (
        <ExploreRoomPanel
          account={account}
          actions={actions}
          detail={panel.detail}
          filter={filter}
          now={now}
          onClose={panel.onClose}
        />
      )}
    </main>
  );
}

/** 没登录时的顶栏：名字、语言、GitHub 和下载，再是“登录”“注册”。 */
function ExploreGuestHeader({
  account,
}: {
  readonly account: Extract<ExploreAccount, { kind: 'signed-out' }>;
}) {
  const { t } = useTranslation();
  return (
    <header className="app-navigation explore__guest-header">
      <div className="app-navigation__identity">
        <Link aria-label={t('app.name')} className="app-navigation__brand" to="/">
          <img alt="" src="/agent-room-mark.svg" />
          <span>{t('app.name')}</span>
        </Link>
      </div>
      <div className="app-navigation__actions">
        <LanguageControl />
        <WebHeaderLinks />
        <Button onClick={account.onSignIn} tone="ghost">
          {t('landing.login')}
        </Button>
        <Button disabled={!account.registrationOpen} onClick={account.onRegister}>
          {t(account.registrationOpen ? 'landing.register' : 'landing.registrationPending')}
        </Button>
      </div>
    </header>
  );
}

function ExploreFilters({
  filter,
  onFilterChange,
}: {
  readonly filter: ExploreFilter;
  readonly onFilterChange: (filter: ExploreFilter) => void;
}) {
  const { t } = useTranslation();
  const sortId = useId();
  const [draft, setDraft] = useState(filter.q);
  // 地址上的搜索词变了（后退、点了别处的链接），框里跟着变。
  useEffect(() => {
    setDraft(filter.q);
  }, [filter.q]);
  useEffect(() => {
    if (draft.trim() === filter.q.trim()) return undefined;
    const timer = window.setTimeout(() => {
      onFilterChange({ ...filter, q: draft });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      window.clearTimeout(timer);
    };
  }, [draft, filter, onFilterChange]);
  return (
    <section aria-label={t('explore.filters')} className="explore__filters">
      <label className="explore__search">
        <Search aria-hidden="true" />
        <input
          aria-label={t('explore.search')}
          maxLength={EXPLORE_SEARCH_MAX_LENGTH}
          onChange={(event) => {
            setDraft(event.target.value);
          }}
          placeholder={t('explore.search.placeholder')}
          type="search"
          value={draft}
        />
      </label>
      <div className="explore__filter-row">
        <div aria-label={t('explore.filters')} className="explore__chips" role="group">
          {exploreQuickFilters.map((quick) => (
            <button
              aria-pressed={filter.quick === quick}
              className="explore__chip"
              key={quick}
              onClick={() => {
                onFilterChange({ ...filter, quick });
              }}
              type="button"
            >
              {t(`explore.filter.${quick}`)}
            </button>
          ))}
        </div>
        <label className="explore__sort" htmlFor={sortId}>
          <span>{t('explore.sort')}</span>
          <select
            id={sortId}
            onChange={(event) => {
              const sort = exploreSorts.find((value) => value === event.target.value);
              if (sort !== undefined) onFilterChange({ ...filter, sort });
            }}
            value={filter.sort}
          >
            {exploreSorts.map((sort) => (
              <option key={sort} value={sort}>
                {t(`explore.sort.${sort}`)}
              </option>
            ))}
          </select>
        </label>
      </div>
    </section>
  );
}

/** 平台建的公共大厅：登录了直接进，没登录先围观。 */
function LobbyRow({
  lobbies,
  signedIn,
}: {
  readonly lobbies: readonly PublicRoomSummary[];
  readonly signedIn: boolean;
}) {
  const { t } = useTranslation();
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="explore__section">
      <div className="explore__heading-row">
        <h2 className="explore__heading" id={headingId}>
          {t('explore.lobbies')}
        </h2>
        <p>{t('explore.lobbies.detail')}</p>
      </div>
      <ul className="explore__lobbies">
        {lobbies.map((lobby) => (
          <li className="explore-lobby" key={lobby.catalogId}>
            <span aria-hidden="true" className="explore-avatar explore-avatar--lobby">
              {avatarLetter(lobby.name)}
            </span>
            <span className="explore-lobby__text">
              <strong>{lobby.name}</strong>
              <small>{t('explore.lobby.agents', { count: lobby.onlineAgentCount })}</small>
            </span>
            {signedIn ? (
              <Link
                aria-label={`${t('explore.lobby.enter')} · ${lobby.name}`}
                className="ar-button ar-button--compact ar-button--primary"
                params={{ catalogId: lobby.catalogId }}
                search={{}}
                to="/lobby/$catalogId"
              >
                {t('explore.lobby.enter')}
                <ArrowRight aria-hidden="true" />
              </Link>
            ) : lobby.slug === null ? (
              <Link
                aria-label={`${t('explore.lobby.watch')} · ${lobby.name}`}
                className="ar-button ar-button--compact ar-button--ghost"
                search={{}}
                to="/watch"
              >
                <Eye aria-hidden="true" />
                {t('explore.lobby.watch')}
              </Link>
            ) : (
              <Link
                aria-label={`${t('explore.lobby.watch')} · ${lobby.name}`}
                className="ar-button ar-button--compact ar-button--ghost"
                params={{ slug: lobby.slug }}
                search={{}}
                to="/watch/$slug"
              >
                <Eye aria-hidden="true" />
                {t('explore.lobby.watch')}
              </Link>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

function RoomResults({
  account,
  actions,
  filter,
  now,
  onCreatePublicRoom,
  onLoadMore,
  onRetry,
  rooms,
}: {
  readonly account: ExploreAccount;
  readonly actions?: ExploreActions | undefined;
  readonly filter: ExploreFilter;
  readonly now: number;
  readonly onCreatePublicRoom?: (() => void) | undefined;
  readonly onLoadMore: () => void;
  readonly onRetry: () => void;
  readonly rooms: ExploreRoomsState;
}) {
  const { t } = useTranslation();
  if (rooms.kind === 'loading') {
    return (
      <ExploreBoundary
        icon={<LoaderCircle aria-hidden="true" className="explore__spin" />}
        role="status"
        title={t('explore.loading')}
      />
    );
  }
  if (rooms.kind === 'failed') {
    const limited = rooms.failure.code === 'public_rooms.rate_limited';
    return (
      <ExploreBoundary
        action={
          <Button icon={<RefreshCw aria-hidden="true" />} onClick={onRetry} tone="ghost">
            {t('explore.retry')}
          </Button>
        }
        detail={limited ? t('explore.rateLimited') : t('explore.failed.detail')}
        icon={<CircleAlert aria-hidden="true" />}
        role="alert"
        title={t('explore.failed.title')}
      >
        {limited ? null : <code>{rooms.failure.code}</code>}
      </ExploreBoundary>
    );
  }
  if (rooms.rooms.length === 0) {
    return (
      <ExploreBoundary
        action={
          onCreatePublicRoom === undefined ? null : (
            <Button icon={<Plus aria-hidden="true" />} onClick={onCreatePublicRoom}>
              {t('explore.empty.create')}
            </Button>
          )
        }
        detail={t('explore.empty.detail')}
        icon={<Search aria-hidden="true" />}
        role="status"
        title={t('explore.empty.title')}
      />
    );
  }
  return (
    <>
      <ul className="explore__rooms">
        {rooms.rooms.map((room) => (
          <ExploreRoomCard
            account={account}
            actions={actions}
            filter={filter}
            key={room.catalogId}
            now={now}
            room={room}
          />
        ))}
      </ul>
      {rooms.more === 'none' ? null : (
        <div className="explore__more">
          <Button
            disabled={rooms.more === 'loading'}
            icon={
              rooms.more === 'loading' ? (
                <LoaderCircle aria-hidden="true" className="explore__spin" />
              ) : undefined
            }
            onClick={onLoadMore}
            tone="ghost"
          >
            {t('explore.loadMore')}
          </Button>
        </div>
      )}
    </>
  );
}

/**
 * 一间公开房间：头像、名字、房主、简介开头两行、人数、多久以前有人说话、加入方式和加不加密。名字是打开
 * 详情的链接，整张卡片都点得到；右下的按钮在它上面，按你的处境写。
 */
function ExploreRoomCard({
  account,
  actions,
  filter,
  now,
  room,
}: {
  readonly account: ExploreAccount;
  readonly actions?: ExploreActions | undefined;
  readonly filter: ExploreFilter;
  readonly now: number;
  readonly room: ExploreRoom;
}) {
  const { i18n, t } = useTranslation();
  const language = i18n.resolvedLanguage;
  return (
    <li className="explore-room">
      <span
        aria-hidden="true"
        className={`explore-avatar explore-avatar--${avatarTone(room.catalogId)}`}
      >
        {avatarLetter(room.name)}
      </span>
      <div className="explore-room__body">
        <h3>
          <Link
            className="explore-room__open"
            params={{ code: room.shareCode }}
            search={exploreSearchFromFilter(filter)}
            to="/explore/$code"
          >
            {room.name}
          </Link>
        </h3>
        {room.ownerName === '' ? null : (
          <p className="explore-room__owner">{t('explore.card.owner', { name: room.ownerName })}</p>
        )}
        {room.description === '' ? null : (
          <p className="explore-room__description">{room.description}</p>
        )}
        <ul className="explore-room__facts">
          <RoomFact icon={<UsersRound aria-hidden="true" />}>
            {t('explore.card.people', { count: room.memberCount })}
          </RoomFact>
          <RoomFact icon={<Bot aria-hidden="true" />}>
            {t('explore.card.agents', { count: room.agentCount })}
          </RoomFact>
          <RoomFact icon={<Clock3 aria-hidden="true" />}>
            {room.lastMessageAtUnixMs === null
              ? t('explore.card.quiet')
              : t('explore.card.lastSpoke', {
                  time: formatRelativeTime(room.lastMessageAtUnixMs, now, language),
                })}
          </RoomFact>
        </ul>
        <ul className="explore-room__tags">
          <li className={`explore-tag explore-tag--${room.joinMode}`}>
            {t(`privateRooms.listing.joinMode.${room.joinMode}.label`)}
          </li>
          <li className="explore-tag">
            {room.endToEndEncrypted ? <Lock aria-hidden="true" /> : <LockOpen aria-hidden="true" />}
            {t(room.endToEndEncrypted ? 'explore.card.encrypted' : 'explore.card.plain')}
          </li>
        </ul>
      </div>
      <div className="explore-room__action">
        <ExploreRoomActions
          account={account}
          actions={actions}
          filter={filter}
          place="card"
          room={room}
        />
      </div>
    </li>
  );
}

function RoomFact({ children, icon }: { readonly children: ReactNode; readonly icon: ReactNode }) {
  return (
    <li>
      {icon}
      <span>{children}</span>
    </li>
  );
}

function ExploreBoundary({
  action,
  children,
  detail,
  icon,
  role,
  title,
}: {
  readonly action?: ReactNode;
  readonly children?: ReactNode;
  readonly detail?: string;
  readonly icon: ReactNode;
  readonly role: 'alert' | 'status';
  readonly title: string;
}) {
  return (
    <div className="explore__boundary" role={role}>
      <span>{icon}</span>
      <div>
        <h3>{title}</h3>
        {detail === undefined ? null : <p>{detail}</p>}
        {children}
      </div>
      {action}
    </div>
  );
}
