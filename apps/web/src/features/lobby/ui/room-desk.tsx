import { Button, RoomGlyph } from '@agent-room/ui-system';
import { Link } from '@tanstack/react-router';
import { ArrowLeft, Bot, Settings2, Share2, UserPlus, UsersRound } from 'lucide-react';
import { useId, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { RoomKindLabel, type RoomKind } from './room-kind-label';
import { RoomUnreadCount } from './room-unread-count';
import './room-desk.css';

/**
 * 右栏一直在的宽度。和 `room-desk.css` 里三栏的断点是同一个：样式管不了的地方（详情上那个
 * 控件叫“回到列表”还是“关闭”）照它判断。
 */
export const roomDeskSideQuery = '(min-width: 1100px)';

/** 顶栏上打开对话框的一个动作；`null` 是这间房里你用不上它。 */
type DialogAction = { readonly open: boolean; readonly onOpen: () => void } | null;

export type RoomDeskProps = {
  readonly agentCount: number;
  /** 对话、须知和输入框。 */
  readonly children: ReactNode;
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
  readonly onSelectMainRoom: () => void;
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
  /** 左栏里主房间下面的东西（私聊）。没有就不出左栏。 */
  readonly rail: ReactNode;
  readonly roomName: string;
  readonly share: DialogAction;
  /** 右栏：房间里的 Agent 和人。 */
  readonly side: ReactNode;
  readonly view: 'conversation' | 'resources';
};

/**
 * 房间页的对话版面（`specs/redesign/room-page.md`）：顶上一条是房间名和几个动作，下面从左到右是
 * 主房间和私聊、对话、Agent。窄的时候左右两栏收起来，Agent 从顶栏的按钮打开。
 */
export function RoomDesk({
  agentCount,
  children,
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
  onSelectMainRoom,
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
  // 人在私聊里时主房间来了新消息：回主房间的两处各标一个数。
  const unreadElsewhere = !mainRoomSelected && mainRoomUnread > 0;
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
        <div className="room-desk__identity">
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
            {peopleCount === 0 ? null : <span>{t('roomDesk.people', { count: peopleCount })}</span>}
            <span>{t('roomDesk.agents', { count: agentCount })}</span>
          </p>
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
            {t('roomDesk.agentsButton')} {agentCount}
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
              aria-describedby={pendingJoinRequests > 0 ? attentionId : undefined}
              aria-haspopup="dialog"
              aria-label={t('roomSettings.open')}
              className="room-desk__settings ar-icon-button"
              onClick={onOpenSettings}
              title={t('roomSettings.open')}
              type="button"
            >
              <Settings2 aria-hidden="true" />
              {pendingJoinRequests > 0 ? (
                <span className="room-desk__attention" id={attentionId}>
                  <span className="sr-only">
                    {t('privateRooms.joinRequests.attention', { count: pendingJoinRequests })}
                  </span>
                </span>
              ) : null}
            </button>
          )}
        </div>
      </header>
      <div
        className="room-desk__body"
        data-inspecting={inspecting ? 'true' : undefined}
        data-rail={rail === null ? 'false' : 'true'}
      >
        {rail === null ? null : (
          <div className="room-desk__rail">
            <nav aria-label={t('roomDesk.places')} className="room-desk__places">
              <button
                aria-current={mainRoomSelected ? 'page' : undefined}
                aria-describedby={unreadElsewhere ? `${unreadId}-rail` : undefined}
                aria-label={t('roomDesk.mainRoom')}
                className="room-desk__place"
                onClick={onSelectMainRoom}
                type="button"
              >
                <RoomGlyph kind="open" />
                <span>{t('roomDesk.mainRoom')}</span>
                {unreadElsewhere ? (
                  <RoomUnreadCount count={mainRoomUnread} id={`${unreadId}-rail`} />
                ) : null}
              </button>
            </nav>
            {rail}
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
