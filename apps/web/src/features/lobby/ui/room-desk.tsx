import { Button, RoomGlyph } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import {
  ArrowLeft,
  Bot,
  Boxes,
  ChevronDown,
  Ellipsis,
  Search,
  Settings2,
  Share2,
  UserPlus,
  UsersRound,
} from 'lucide-react';
import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { RoomDeskMenu } from './room-desk-menu';
import { RoomKindLabel, roomKindWarns, type RoomKind } from './room-kind-label';
import { RoomUnreadCount } from './room-unread-count';
import './room-desk.css';

/**
 * 右栏一直在的宽度。和 `room-desk.css` 里三栏的断点是同一个：样式管不了的地方（详情上那个
 * 控件叫“回到列表”还是“关闭”）照它判断。
 */
export const roomDeskSideQuery = '(min-width: 1100px)';

/** 手机宽度：顶栏只剩一行，别的动作收进“更多”。和 `room-desk.css` 里的断点是同一个。 */
export const roomDeskPhoneQuery = '(max-width: 767px)';

/** 顶栏上打开对话框的一个动作；`null` 是这间房里你用不上它。 */
type DialogAction = { readonly open: boolean; readonly onOpen: () => void } | null;

export type RoomDeskProps = {
  readonly agentCount: number;
  /** 对话、须知和输入框。 */
  readonly children: ReactNode;
  /** 人在私聊里时对面叫什么，不在私聊里是 `null`。左栏收起来时写在房间名旁边的下拉上。 */
  readonly directName: string | null;
  /** 选中 Agent 时盖在右栏上的详情。 */
  readonly inspector: ReactNode;
  /** 详情是不是开着：开着时右栏换成它，下面那张表格读屏和键盘都碰不到。 */
  readonly inspecting: boolean;
  readonly inviteAgent: DialogAction;
  readonly invitePeople: DialogAction;
  readonly kind: RoomKind;
  readonly layoutSwitch: ReactNode;
  readonly liveSummary: string;
  /** 人是不是在主房间的对话里：在私聊里、在看展开的那一页时都不是。 */
  readonly mainRoomSelected: boolean;
  /** 主房间里你还没看到的消息数：人不在主房间的对话里时标在回去的地方。 */
  readonly mainRoomUnread: number;
  readonly onOpenAgents: () => void;
  readonly onOpenSettings: (() => void) | null;
  /**
   * 打开眼前这段对话的查找，手机上“更多”里那一项用（宽的时候按钮就在对话顶上那一行）。
   * `null` 是此刻没有可查的：一条消息都没有，或者眼前不是对话。
   */
  readonly onSearch: (() => void) | null;
  readonly onSelectMainRoom: () => void;
  /** 换到场景版面，手机上“更多”里那一项用（宽的时候是 `layoutSwitch`）。 */
  readonly onShowScene: () => void;
  /**
   * 展开成整页的东西（Agent 表格）：有的时候它占着主栏和右栏，对话和右栏都不画，
   * 顶栏的房间名变成“房间名 / 这一页的名字”，点房间名回到对话（`onClose`）。
   */
  readonly page: {
    readonly title: string;
    readonly content: ReactNode;
    readonly onClose: () => void;
  } | null;
  readonly peopleCount: number;
  readonly pendingJoinRequests: number;
  /**
   * 主房间下面的东西（私聊）；没有就不出左栏。宽的时候它在左栏，窄了收进房间名旁边的下拉：
   * 在下拉里点了一项要把下拉关上，所以画的时候给它一个 `onNavigate`。
   */
  readonly rail: ((onNavigate: () => void) => ReactNode) | null;
  readonly roomName: string;
  readonly share: DialogAction;
  /** 右栏：房间里的 Agent 和人。 */
  readonly side: ReactNode;
  readonly view: 'conversation' | 'resources';
};

const stay = (): void => undefined;

/**
 * 房间页的对话版面（`specs/redesign/room-page.md`）：顶上一条是房间名和几个动作，下面从左到右是
 * 主房间和私聊、对话、Agent。
 * - 1100px 以上三栏。
 * - 窄一些：右栏收成顶栏上的“Agent N”，左栏收进房间名旁边的下拉。
 * - 手机：顶栏只有一行，回到房间列表、在哪儿（下拉）、“Agent N”、“更多”；别的动作都在“更多”里。
 */
export function RoomDesk({
  agentCount,
  children,
  directName,
  inspector,
  inspecting,
  inviteAgent,
  invitePeople,
  kind,
  layoutSwitch,
  liveSummary,
  mainRoomSelected,
  mainRoomUnread,
  onOpenAgents,
  onOpenSettings,
  onSearch,
  onSelectMainRoom,
  onShowScene,
  page,
  peopleCount,
  pendingJoinRequests,
  rail,
  roomName,
  share,
  side,
  view,
}: RoomDeskProps) {
  const { t } = useTranslation();
  const attentionId = useId();
  const unreadId = useId();
  // 人在私聊里时主房间来了新消息：回主房间的几处各标一个数。
  const unreadElsewhere = !mainRoomSelected && mainRoomUnread > 0;
  const [menu, setMenu] = useState<'places' | 'more' | null>(null);
  const placesTrigger = useRef<HTMLButtonElement>(null);
  const moreTrigger = useRef<HTMLButtonElement>(null);
  const closeMenu = (): void => {
    setMenu(null);
  };
  // 这两块都是窄的时候才有的：窗口拉过了断点，打开它的按钮就没了，跟着收起来，
  // 不然页面还被一层说不清从哪来的东西挡着。
  useEffect(() => {
    if (menu === null || typeof window.matchMedia !== 'function') return;
    const lists = [window.matchMedia(roomDeskSideQuery), window.matchMedia(roomDeskPhoneQuery)];
    const close = (): void => {
      setMenu(null);
    };
    for (const list of lists) list.addEventListener('change', close);
    return () => {
      for (const list of lists) list.removeEventListener('change', close);
    };
  }, [menu]);
  // 下拉只在对话上有：展开成整页时，顶栏那里写的是“房间名 / 这一页”，点房间名回对话。
  const placeMenu = rail !== null && page === null;
  const place = directName ?? t('roomDesk.mainRoom');
  const mainRoomPlace = (where: 'rail' | 'menu', onNavigate: () => void) => (
    <nav aria-label={t('roomDesk.places')} className="room-desk__places">
      <button
        aria-current={mainRoomSelected ? 'page' : undefined}
        aria-describedby={unreadElsewhere ? `${unreadId}-${where}` : undefined}
        aria-label={t('roomDesk.mainRoom')}
        className="room-desk__place"
        onClick={() => {
          onNavigate();
          onSelectMainRoom();
        }}
        type="button"
      >
        <RoomGlyph kind="open" />
        <span>{t('roomDesk.mainRoom')}</span>
        {unreadElsewhere ? (
          <RoomUnreadCount count={mainRoomUnread} id={`${unreadId}-${where}`} />
        ) : null}
      </button>
    </nav>
  );
  // “更多”里点了一项：先收起来（焦点回到“更多”），再做那件事；打开的对话框关掉以后焦点也回到“更多”。
  const pick = (action: () => void) => (): void => {
    setMenu(null);
    action();
  };
  const waiting =
    pendingJoinRequests > 0
      ? t('privateRooms.joinRequests.attention', { count: pendingJoinRequests })
      : null;
  return (
    <main
      className="lobby-workspace room-desk"
      data-layout="conversation"
      data-page={page === null ? undefined : 'true'}
      data-view={view}
      id="main-content"
    >
      <p aria-atomic="true" aria-live="polite" className="sr-only">
        {liveSummary}
      </p>
      <header className="room-desk__top">
        <Link
          aria-label={t('roomWorkspace.backToRooms')}
          className="room-desk__back ar-icon-button"
          title={t('roomWorkspace.backToRooms')}
          to="/rooms"
        >
          <ArrowLeft aria-hidden="true" />
        </Link>
        <div className="room-desk__identity" data-places={placeMenu ? 'true' : undefined}>
          {/* 房间名和那一小句。宽的时候这一层不算数（`display: contents`），下拉排在两样中间；
              手机上它是下拉下面的那一行。 */}
          <div className="room-desk__titles" data-warns={roomKindWarns(kind) ? 'true' : undefined}>
            {page === null ? (
              <h1>{roomName}</h1>
            ) : (
              <h1 className="room-desk__crumbs">
                <button
                  aria-describedby={unreadElsewhere ? `${unreadId}-crumb` : undefined}
                  aria-label={t('roomRoster.page.back', { room: roomName })}
                  onClick={page.onClose}
                  type="button"
                >
                  {roomName}
                </button>
                {unreadElsewhere ? (
                  <RoomUnreadCount count={mainRoomUnread} id={`${unreadId}-crumb`} />
                ) : null}
                <span aria-hidden="true">/</span>
                <span aria-current="page">{page.title}</span>
              </h1>
            )}
            <p className="room-desk__meta">
              <RoomKindLabel {...kind} />
              {peopleCount === 0 ? null : (
                <span>{t('roomDesk.people', { count: peopleCount })}</span>
              )}
              <span>{t('roomDesk.agents', { count: agentCount })}</span>
            </p>
          </div>
          {placeMenu ? (
            <>
              <span aria-hidden="true" className="room-desk__place-slash">
                /
              </span>
              <button
                aria-describedby={unreadElsewhere ? `${unreadId}-place` : undefined}
                aria-expanded={menu === 'places'}
                aria-haspopup="dialog"
                aria-label={t('roomDesk.placesOpen', { place })}
                className="room-desk__place-trigger"
                onClick={() => {
                  setMenu('places');
                }}
                ref={placesTrigger}
                type="button"
              >
                <span>{place}</span>
                <ChevronDown aria-hidden="true" />
                {unreadElsewhere ? (
                  <RoomUnreadCount count={mainRoomUnread} id={`${unreadId}-place`} />
                ) : null}
              </button>
            </>
          ) : null}
        </div>
        <div className="room-desk__actions">
          {layoutSwitch}
          {share === null ? null : (
            <Button
              aria-expanded={share.open}
              aria-haspopup="dialog"
              className="room-desk__action"
              icon={<Share2 aria-hidden="true" />}
              onClick={share.onOpen}
              size="compact"
              tone="quiet"
            >
              {t('roomShare.open')}
            </Button>
          )}
          {invitePeople === null ? null : (
            <Button
              aria-expanded={invitePeople.open}
              aria-haspopup="dialog"
              className="room-desk__action"
              icon={<UserPlus aria-hidden="true" />}
              onClick={invitePeople.onOpen}
              size="compact"
              tone="quiet"
            >
              {t('privateRooms.inviteLink.open')}
            </Button>
          )}
          <Button
            aria-label={t('roomWorkspace.openMembers')}
            className="room-desk__agents-button"
            icon={<UsersRound aria-hidden="true" />}
            onClick={onOpenAgents}
            size="compact"
            tone="quiet"
          >
            <span className="room-desk__agents-word">{t('roomDesk.agentsButton')}</span>{' '}
            {agentCount}
          </Button>
          {inviteAgent === null ? null : (
            <Button
              aria-expanded={inviteAgent.open}
              aria-haspopup="dialog"
              className="room-desk__action"
              icon={<Bot aria-hidden="true" />}
              onClick={inviteAgent.onOpen}
              size="compact"
              tone="primary"
            >
              {t('agentInvite.open')}
            </Button>
          )}
          {onOpenSettings === null ? null : (
            <button
              aria-describedby={waiting === null ? undefined : attentionId}
              aria-haspopup="dialog"
              aria-label={t('roomSettings.open')}
              className="room-desk__settings ar-icon-button"
              onClick={onOpenSettings}
              title={t('roomSettings.open')}
              type="button"
            >
              <Settings2 aria-hidden="true" />
              {waiting === null ? null : (
                <span className="room-desk__attention" id={attentionId}>
                  <span className="sr-only">{waiting}</span>
                </span>
              )}
            </button>
          )}
          <button
            aria-describedby={waiting === null ? undefined : `${attentionId}-more`}
            aria-expanded={menu === 'more'}
            aria-haspopup="dialog"
            aria-label={t('roomDesk.more')}
            className="room-desk__more ar-icon-button"
            onClick={() => {
              setMenu('more');
            }}
            ref={moreTrigger}
            title={t('roomDesk.more')}
            type="button"
          >
            <Ellipsis aria-hidden="true" />
            {waiting === null ? null : (
              <span className="room-desk__attention" id={`${attentionId}-more`}>
                <span className="sr-only">{waiting}</span>
              </span>
            )}
          </button>
        </div>
      </header>
      {menu === 'places' && rail !== null ? (
        <RoomDeskMenu anchor={placesTrigger} label={t('roomDesk.places')} onClose={closeMenu}>
          {mainRoomPlace('menu', closeMenu)}
          {rail(closeMenu)}
        </RoomDeskMenu>
      ) : null}
      {menu === 'more' ? (
        <RoomDeskMenu anchor={moreTrigger} label={t('roomDesk.more')} onClose={closeMenu}>
          <ul className="room-desk-menu__items">
            {onSearch === null ? null : (
              <MenuItem
                icon={<Search aria-hidden="true" />}
                label={t('history.search')}
                onPick={pick(onSearch)}
              />
            )}
            {share === null ? null : (
              <MenuItem
                icon={<Share2 aria-hidden="true" />}
                label={t('roomShare.open')}
                onPick={pick(share.onOpen)}
                opensDialog
              />
            )}
            {invitePeople === null ? null : (
              <MenuItem
                icon={<UserPlus aria-hidden="true" />}
                label={t('privateRooms.inviteLink.open')}
                onPick={pick(invitePeople.onOpen)}
                opensDialog
              />
            )}
            {inviteAgent === null ? null : (
              <MenuItem
                icon={<Bot aria-hidden="true" />}
                label={t('agentInvite.open')}
                onPick={pick(inviteAgent.onOpen)}
                opensDialog
              />
            )}
            {onOpenSettings === null ? null : (
              <MenuItem
                icon={<Settings2 aria-hidden="true" />}
                label={t('roomSettings.open')}
                note={waiting}
                onPick={pick(onOpenSettings)}
                opensDialog
              />
            )}
            <MenuItem
              icon={<Boxes aria-hidden="true" />}
              label={t('roomWorkspace.layout.toScene')}
              onPick={pick(onShowScene)}
            />
          </ul>
        </RoomDeskMenu>
      ) : null}
      <div
        className="room-desk__body"
        data-inspecting={inspecting ? 'true' : undefined}
        data-rail={rail === null ? 'false' : 'true'}
      >
        {rail === null ? null : (
          <div className="room-desk__rail">
            {mainRoomPlace('rail', stay)}
            {rail(stay)}
          </div>
        )}
        {page === null ? (
          <>
            <section aria-label={t('roomGame.panel')} className="room-desk__main">
              {mainRoomSelected ? null : (
                <button
                  aria-describedby={unreadElsewhere ? `${unreadId}-return` : undefined}
                  aria-label={t('roomDesk.backToMainRoom')}
                  className="room-desk__return"
                  onClick={onSelectMainRoom}
                  type="button"
                >
                  <ArrowLeft aria-hidden="true" />
                  <span>{t('roomDesk.backToMainRoom')}</span>
                  {unreadElsewhere ? (
                    <RoomUnreadCount count={mainRoomUnread} id={`${unreadId}-return`} />
                  ) : null}
                </button>
              )}
              {children}
            </section>
            <aside aria-label={t('roomWorkspace.members')} className="room-desk__side">
              {side}
            </aside>
          </>
        ) : (
          // 对话和右栏这时不画：回去的时候对话照上次读到的地方重新打开，草稿存在页面外面。
          <div className="room-desk__page">{page.content}</div>
        )}
        {inspector}
      </div>
    </main>
  );
}

/**
 * “更多”里的一项。`note` 是要人留意的事（有人在等批准）：这一项后面带一个点，
 * 话经 `aria-describedby` 读出来，按钮的名字不变。
 */
function MenuItem({
  icon,
  label,
  note = null,
  onPick,
  opensDialog = false,
}: {
  readonly icon: ReactNode;
  readonly label: string;
  readonly note?: string | null;
  readonly onPick: () => void;
  readonly opensDialog?: boolean;
}) {
  const noteId = useId();
  return (
    <li>
      <button
        aria-describedby={note === null ? undefined : noteId}
        aria-haspopup={opensDialog ? 'dialog' : undefined}
        aria-label={label}
        className="room-desk-menu__item"
        onClick={onPick}
        type="button"
      >
        {icon}
        <span>{label}</span>
        {note === null ? null : (
          <span className="room-desk-menu__attention" id={noteId}>
            <span className="sr-only">{note}</span>
          </span>
        )}
      </button>
    </li>
  );
}
