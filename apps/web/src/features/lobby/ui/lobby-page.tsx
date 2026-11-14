import { usePublishedDownload } from '@/features/updates/ui/use-published-download';
import {
  Archive,
  Bot,
  Files,
  Maximize2,
  MessageCircle,
  MessageSquareOff,
  Minimize2,
  Share2,
  UserPlus,
  UsersRound,
  X,
} from 'lucide-react';
import { AnimatePresence } from 'motion/react';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';
import { useAppServices } from '@/app/app-services';
import { ConversationWorkspaceProvider } from '@/features/conversation/ui/conversation-workspace-context';
import { AgentInviteDialog } from '@/features/desktop/ui/agent-invite-dialog';
import { ReceptionPanel } from '@/features/desktop/ui/reception-panel';
import { ReceptionOwnershipPanel } from '@/features/desktop/ui/reception-ownership-panel';
import { InviteReplyProgress } from '@/features/desktop/ui/invite-reply-progress';
import type { ConnectedInvitation } from '@/features/desktop/domain/invite-reply';
import {
  ReceptionEvidenceProvider,
  useReceptionEvidence,
} from '@/features/desktop/ui/reception-evidence-context';
import { hasAutomationDraftFor } from '@/features/automation/ui/automation-settings';
import { DirectConversationDock } from '@/features/direct-sessions/ui/direct-conversation-dock';
import {
  useDirectSessionController,
  type DirectSessionController,
} from '@/features/direct-sessions/ui/use-direct-session-controller';
import { LobbyExperienceStore } from '@/features/lobby/application/lobby-experience-store';
import { RoomActivityStore } from '@/features/lobby/application/room-activity-store';
import {
  RoomMessagesProvider,
  useRoomMessages,
} from '@/features/messages/ui/room-messages-context';
import { projectRoomSpeech } from '@/features/lobby/domain/room-speech';
import type { LobbyRoom } from '@/features/lobby/domain/lobby';
import { attendanceCounts } from '@/features/lobby/domain/agent-attendance';
import type { RoomPageLayout, RoomWorkspaceView } from '@/features/lobby/domain/workspace-view';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';
import { AgentInspector, type AgentInspectorFailure } from '@/features/lobby/ui/agent-inspector';
import { AgentRosterPolicyPanel } from './agent-roster-policy-panel';
import { ListModeRoster } from '@/features/lobby/ui/list-mode-roster';
import { RoomRoster } from '@/features/lobby/ui/room-roster';
import { useRoomRoster } from '@/features/lobby/ui/use-room-roster';
import { useMediaQuery } from '@/shared/ui/use-media-query';
import {
  RoomSettingsDialog,
  type RoomSettingsSection,
} from '@/features/lobby/ui/room-settings-dialog';
import {
  LobbySpatialView,
  type LobbySpatialViewHandle,
} from '@/features/lobby/ui/lobby-spatial-view';
import { LobbyStateBoundary } from '@/features/lobby/ui/lobby-state-boundary';
import { PrivateRoomJoinBoundary } from '@/features/lobby/ui/private-room-join-boundary';
import { MatrixConnectionCard } from '@/features/session/ui/matrix-connection-card';
import { AgentOwnersScope } from '@/features/lobby/ui/agent-owners';
import { NetworkAgentRelayProvider } from '@/features/lobby/ui/network-agent-labels';
import { allows, memberFor, type PrivateRoom } from '@/features/private-rooms/domain/private-room';
import { PrivateRoomInviteDialog } from '@/features/private-rooms/ui/private-room-invite-dialog';
import { RoomNoticeBar } from '@/features/room-notice/ui/room-notice-bar';
import { usePendingJoinRequestCount } from '@/features/private-rooms/ui/join-request-attention';
import { RoomBeacon } from '@/features/lobby/ui/room-beacon';
import { RoomDesk, roomDeskSideQuery } from '@/features/lobby/ui/room-desk';
import { RoomLayoutSwitch } from '@/features/lobby/ui/room-layout-switch';
import { WorkspaceDrawer } from '@/features/lobby/ui/workspace-drawer';
import { DirectSessionNavigation } from '@/features/direct-sessions/ui/direct-session-navigation';
import { WorkspaceViewTabs } from '@/features/lobby/ui/workspace-view-tabs';
import { MessageLayer } from '@/features/messages/ui/message-layer';
import {
  usePrivateRoomList,
  usePrivateRoomListing,
  usePrivateRoomOrigin,
} from '@/features/private-rooms/data/private-room-queries';
import { shareable } from '@/features/private-rooms/domain/private-room-listing';
import { ShareRoomDialog } from '@/features/room-share/ui/share-room-dialog';
import type { ShareableRoom } from '@/features/room-share/domain/room-share';
import type { WebSession } from '@/features/session/domain/session';
import './lobby-workspace.css';
import './lobby-game.css';

// 对话面板放大与否只影响这台设备的布局，存在本机；读写失败（隐私模式等）就只在本次有效。
const panelSizeKey = 'agent-room.room-panel-size';
/** 还没有人说过话时用的同一份空名单：每次重画都新建一份的话，右栏的人会跟着重算。 */
const noHumans: readonly never[] = [];
type PanelSize = 'regular' | 'wide';

function readPanelSize(): PanelSize {
  try {
    return window.localStorage.getItem(panelSizeKey) === 'wide' ? 'wide' : 'regular';
  } catch {
    return 'regular';
  }
}

function writePanelSize(size: PanelSize): void {
  try {
    window.localStorage.setItem(panelSizeKey, size);
  } catch {
    // 只在本次有效。
  }
}

export type LobbyPageProps = {
  readonly catalogId: string;
  /** 对话占主栏，还是原来的场景（`specs/redesign/room-page.md`）。 */
  readonly layout: RoomPageLayout;
  readonly onExitRoom: () => void;
  readonly onLayoutChange: (layout: RoomPageLayout) => void;
  readonly onSelectedAgentChange: (agentId: string | null) => void;
  readonly onSelectedDirectSessionChange: (catalogId: string | null) => void;
  readonly onSelectedMessageChange: (messageId: string | null) => void;
  readonly onViewChange: (view: RoomWorkspaceView) => void;
  readonly onOpenRoomPanel: (view: 'conversation' | 'resources') => void;
  readonly principal: WebSession | null;
  readonly roomId: string;
  readonly selectedAgentId: string | null;
  readonly selectedDirectSessionId: string | null;
  readonly selectedMessageId: string | null;
  readonly view: RoomWorkspaceView;
};

export function LobbyPage(props: LobbyPageProps) {
  const { lobby, messages, messagePublisher, localRuntime } = useAppServices();
  const store = useMemo(
    () => new LobbyExperienceStore(lobby, messages, props.roomId, props.principal),
    [lobby, messages, props.roomId, props.principal?.matrixUserId, props.principal?.displayName],
  );
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  return (
    <ReceptionEvidenceProvider
      gateway={localRuntime}
      principalId={props.principal?.principalId ?? null}
    >
      <ConversationWorkspaceProvider
        publisher={messagePublisher}
        scope={props.principal?.matrixUserId ?? null}
      >
        {state.kind !== 'ready' ? (
          state.kind === 'failed' && state.code === 'lobby.matrix_unavailable' ? (
            // 消息没连上时照会话的状态说、按钮交给会话；连上以后房间自己打开。
            <MatrixConnectionCard
              fallback={<LobbyStateBoundary onRetry={store.retry} state={state} />}
            />
          ) : state.kind === 'failed' && state.code === 'lobby.room_not_joined' ? (
            // 服务器记着你在、Matrix 上还只是受邀（本机 Agent 替你建的房间）：替你加入，不让你回列表找“加入”。
            <PrivateRoomJoinBoundary
              catalogId={props.catalogId}
              onRetry={store.retry}
              principalId={props.principal?.principalId ?? null}
              roomId={props.roomId}
              state={state}
            />
          ) : (
            <LobbyStateBoundary onRetry={store.retry} state={state} />
          )
        ) : (
          <RoomMessagesProvider store={store.messages}>
            <NetworkAgentRelayProvider relayed={state.room.encrypted === true}>
              <AgentOwnersScope
                catalogId={props.catalogId}
                selfPrincipalId={props.principal?.principalId ?? null}
              >
                <ReadyLobby
                  {...props}
                  key={state.room.roomId}
                  room={state.room}
                  scene={state.projection}
                />
              </AgentOwnersScope>
            </NetworkAgentRelayProvider>
          </RoomMessagesProvider>
        )}
      </ConversationWorkspaceProvider>
    </ReceptionEvidenceProvider>
  );
}

function ReadyLobby({
  catalogId,
  layout,
  onExitRoom,
  onLayoutChange,
  onSelectedAgentChange,
  onSelectedDirectSessionChange,
  onSelectedMessageChange,
  onViewChange,
  onOpenRoomPanel,
  principal,
  room,
  scene,
  selectedAgentId,
  selectedDirectSessionId,
  selectedMessageId,
  view,
}: LobbyPageProps & { readonly room: LobbyRoom; readonly scene: LobbySceneProjection }) {
  const { t } = useTranslation();
  const {
    config,
    localRuntime,
    messages: messageGateway,
    privateRoomInvites,
    privateRoomListings,
    privateRooms,
    roomNotices,
  } = useAppServices();
  const reception = useReceptionEvidence();
  const { url: downloadUrl } = usePublishedDownload(config);
  const directSessions = useDirectSessionController(principal !== null);
  const [drawer, setDrawer] = useState<'members' | null>(null);
  // 重新登录回来时，没提交完的自动发言授权要接着填：直接打开房间设置的那一节。
  const [roomSettings, setRoomSettings] = useState<{
    readonly section?: RoomSettingsSection;
  } | null>(() =>
    principal !== null && hasAutomationDraftFor(principal.principalId, catalogId)
      ? { section: 'automation' }
      : null,
  );
  const [inviteOpen, setInviteOpen] = useState(false);
  const [peopleInviteOpen, setPeopleInviteOpen] = useState(false);
  const [shareOpen, setShareOpen] = useState(false);
  const [invitation, setInvitation] = useState<ConnectedInvitation | null>(null);
  const [panelSize, setPanelSize] = useState<PanelSize>(readPanelSize);
  const owner =
    principal === null
      ? null
      : { principalId: principal.principalId, displayName: principal.displayName };
  const membersButton = useRef<HTMLButtonElement>(null);
  const spatial = useRef<LobbySpatialViewHandle>(null);
  // 归档了的私人房间照样打得开、看得到之前的消息，只是谁都不能再说话、也不能再接入 Agent。
  const privateRoomList = usePrivateRoomList(
    privateRooms,
    principal?.principalId,
    principal !== null,
  );
  // 有房间号的房间（私人房间、公开房间）和公共大厅分开：加密的都是；不加密的公开房间在你的房间里，
  // 公共大厅不在（`specs/public-rooms/design.md`）。
  const catalogRoom =
    room.encrypted === true ||
    (privateRoomList.data?.ok === true &&
      privateRoomList.data.value.some((entry) => entry.catalogId === catalogId));
  const origin = usePrivateRoomOrigin(privateRooms, catalogId, principal !== null && catalogRoom);
  const listing = usePrivateRoomListing(
    privateRoomListings,
    catalogId,
    principal !== null && catalogRoom,
  );
  const archived =
    privateRoomList.data?.ok === true &&
    privateRoomList.data.value.some(
      (entry) => entry.catalogId === catalogId && entry.status === 'archived',
    );
  // 私人房间里你没有“自动发言”时，带进来的 Agent 进得来、说不了话：“接入 Agent”里先说。
  const agentsCanSpeak = agentsSpeakIn(
    privateRoomList.data?.ok === true ? privateRoomList.data.value : [],
    catalogId,
    principal?.principalId ?? null,
  );
  const invitable = invitableRoom(
    privateRoomList.data?.ok === true ? privateRoomList.data.value : [],
    catalogId,
    privateRoomInvites === undefined ? null : (principal?.principalId ?? null),
  );
  // 公开房间（不是只能邀请的、没被平台拿掉）给“分享”：链接打开了别人进得来才给。
  const shareRoom: ShareableRoom | null =
    !archived && listing.data?.ok === true && shareable(listing.data.value)
      ? {
          description:
            (privateRoomList.data?.ok === true
              ? privateRoomList.data.value.find((entry) => entry.catalogId === catalogId)
                  ?.description
              : undefined) ?? '',
          name: room.name,
          shareUrl: listing.data.value.shareUrl,
        }
      : null;
  const pendingJoinRequests = usePendingJoinRequestCount(
    privateRoomList.data?.ok === true ? privateRoomList.data.value : [],
    catalogId,
    principal?.principalId ?? null,
  );
  // 能不能发消息看 Matrix 的发言门槛：归档、被禁言、只能查看时输入框都关着，免得发了才说失败。
  const writesAllowed = !archived && room.canSpeak !== false;
  // 房主和管理员能改公告；归档了谁都不能。
  const noticeEditable =
    roomNotices !== undefined &&
    !archived &&
    managesRoom(
      privateRoomList.data?.ok === true ? privateRoomList.data.value : [],
      catalogId,
      principal?.principalId ?? null,
    );
  const projection = useMemo(
    () => ({
      ...scene,
      selectedAgentId: scene.nodes.some((agent) => agent.agentId === selectedAgentId)
        ? selectedAgentId
        : null,
    }),
    [scene, selectedAgentId],
  );
  const selectedAgent = room.agents.find((agent) => agent.agentId === selectedAgentId) ?? null;
  const attendance = attendanceCounts(room.agents, room.observedAtUnixMs);
  // 对话版面里对话一直在：地址里不带 `view` 也是对话。场景里不带 `view` 是画面本身，私聊开着时才算对话。
  const conversationLayout = layout === 'conversation';
  // 右栏的表格和顶栏那一行小字用同一份数。
  const roster = useRoomRoster({
    agents: room.agents,
    catalogId,
    observedAtUnixMs: room.observedAtUnixMs,
    principalId: principal?.principalId ?? null,
    room:
      privateRoomList.data?.ok === true
        ? privateRoomList.data.value.find((entry) => entry.catalogId === catalogId)
        : undefined,
    roomId: room.roomId,
    spoken: projection.humans ?? noHumans,
  });
  const sideColumn = useMediaQuery(roomDeskSideQuery);
  const activeView =
    view === 'space' && (conversationLayout || selectedDirectSessionId !== null)
      ? 'conversation'
      : view;
  const { store: messageStore } = useRoomMessages(room.roomId);
  const activityStore = useMemo(
    () => new RoomActivityStore(messageStore, principal?.matrixUserId ?? null),
    [messageStore, principal?.matrixUserId],
  );
  const activity = useSyncExternalStore(
    activityStore.subscribe,
    activityStore.getSnapshot,
    activityStore.getSnapshot,
  );
  const speech = useMemo(
    () => projectRoomSpeech(projection, activity.recent),
    [projection, activity.recent],
  );
  const publicConversationVisible =
    activeView === 'conversation' && selectedDirectSessionId === null;
  const [focusedConversationMessageId, setFocusedConversationMessageId] = useState<string | null>(
    null,
  );
  useEffect(() => {
    activityStore.setVisible(publicConversationVisible);
  }, [activityStore, publicConversationVisible]);
  const openRoomPanel = (nextView: 'conversation' | 'resources'): void => {
    setFocusedConversationMessageId(null);
    onOpenRoomPanel(nextView);
  };
  const openSpeech = (messageId: string): void => {
    onOpenRoomPanel('conversation');
    setFocusedConversationMessageId(messageId);
  };
  const panelView = activeView === 'resources' ? 'resources' : 'conversation';
  useEffect(() => {
    if (selectedAgentId !== null && selectedAgent === null) onSelectedAgentChange(null);
  }, [onSelectedAgentChange, selectedAgent, selectedAgentId]);

  const closeDrawer = (): void => {
    setDrawer(null);
  };
  const roomKind = {
    archived,
    listed: listing.data?.ok === true && listing.data.value.listed,
    privateRoom: catalogRoom,
    serverReadable: origin.data?.ok === true && origin.data.value.ownerIsNetworkAgent,
    unencrypted: catalogRoom && room.encrypted !== true,
  };
  // 换版面以后顶栏整个换了一份：焦点接着落在新顶栏的切换上，不掉回页面开头。
  const layoutChangedHere = useRef(false);
  const changeLayout = (next: RoomPageLayout): void => {
    layoutChangedHere.current = true;
    closeDrawer();
    onLayoutChange(next);
  };
  useEffect(() => {
    if (!layoutChangedHere.current) return;
    layoutChangedHere.current = false;
    const controls = document.querySelectorAll<HTMLElement>(
      '.room-layout-switch [aria-checked="true"], .room-layout-toggle',
    );
    [...controls].find((control) => control.offsetParent !== null)?.focus();
  }, [layout]);
  const selectAgent = (id: string | null): void => {
    closeDrawer();
    onSelectedAgentChange(id);
  };
  // 对话版面里关掉 Agent 详情：焦点回到右栏里刚才那一行（那一行不在这一页上就回到标签）；
  // 右栏收起来了就回到顶栏打开它的按钮。详情开着时表格碰不到，所以等它关了、页面重画完再放焦点。
  const returnFocusFrom = useRef<string | null>(null);
  useEffect(() => {
    const agentId = returnFocusFrom.current;
    if (agentId === null || selectedAgentId !== null) return;
    returnFocusFrom.current = null;
    const origins = [
      ...document.querySelectorAll<HTMLElement>('.room-desk__side .agent-table__name'),
    ].filter((row) => row.dataset.agentId === agentId);
    origins.push(
      ...document.querySelectorAll<HTMLElement>(
        '.room-desk__side [role="tab"][aria-selected="true"], .room-desk__agents-button',
      ),
    );
    origins.find((origin) => origin.offsetParent !== null)?.focus();
  }, [selectedAgentId]);
  const directNavigation =
    principal === null ? null : (
      <DirectSessionNavigation
        activeCatalogId={selectedDirectSessionId}
        controller={directSessions}
        onActivate={(id) => {
          closeDrawer();
          onSelectedDirectSessionChange(id);
        }}
      />
    );
  const rosterPolicy = (
    <AgentRosterPolicyPanel
      key={`${catalogId}:${String(room.archiveAfterDays ?? 7)}`}
      catalogId={catalogId}
      days={room.archiveAfterDays ?? 7}
    />
  );
  // 抽屉里连私聊一起列；对话版面的右栏只列 Agent 和人，私聊在左栏。
  // 对话版面是表格（`RoomRoster`）；场景里照旧是原来那份紧凑的名册。
  const rosterWith = (directMessages: boolean) =>
    conversationLayout ? (
      <div className="workspace-members">
        <RoomRoster
          agents={room.agents}
          footer={rosterPolicy}
          observedAtUnixMs={room.observedAtUnixMs}
          onInviteAgent={
            archived
              ? null
              : () => {
                  closeDrawer();
                  setInviteOpen(true);
                }
          }
          onSelectAgent={selectAgent}
          people={roster.people}
          selectedAgentId={selectedAgentId}
          wakeableAgentIds={roster.wakeableAgentIds}
        />
        {directMessages ? directNavigation : null}
      </div>
    ) : (
      <div className="workspace-members">
        <ListModeRoster
          agents={room.agents}
          observedAtUnixMs={room.observedAtUnixMs}
          onSelectAgent={selectAgent}
          selectedAgentId={selectedAgentId}
          variant="compact"
        />
        <p className="workspace-members__note">{t('roomWorkspace.memberNote')}</p>
        {directMessages ? directNavigation : null}
        {rosterPolicy}
      </div>
    );
  const layoutSwitch = <RoomLayoutSwitch onChange={changeLayout} value={layout} />;
  const dialogs = (
    <>
      {shareOpen && shareRoom !== null ? (
        <ShareRoomDialog
          onClose={() => {
            setShareOpen(false);
          }}
          room={shareRoom}
        />
      ) : null}
      {roomSettings === null || principal === null ? null : (
        <RoomSettingsDialog
          catalogId={catalogId}
          initialSection={roomSettings.section}
          onClose={() => {
            setRoomSettings(null);
          }}
          notice={room.notice}
          onExitRoom={onExitRoom}
          principal={principal}
          roomName={room.name}
        />
      )}
      {peopleInviteOpen &&
      invitable !== null &&
      principal !== null &&
      privateRoomInvites !== undefined ? (
        <PrivateRoomInviteDialog
          invites={privateRoomInvites}
          onClose={() => {
            setPeopleInviteOpen(false);
          }}
          principalId={principal.principalId}
          room={invitable}
          rooms={privateRooms}
        />
      ) : null}
      {inviteOpen ? (
        <AgentInviteDialog
          agentsCanSpeak={agentsCanSpeak}
          onConnected={setInvitation}
          onStartConversation={() => {
            setInviteOpen(false);
            onOpenRoomPanel('conversation');
          }}
          downloadUrl={downloadUrl}
          onClose={() => {
            setInviteOpen(false);
          }}
          owner={owner}
          presentAgents={room.agents}
          room={{ catalogId, roomId: room.roomId, roomName: room.name }}
        />
      ) : null}
    </>
  );
  const conversationNotices = (
    <>
      {writesAllowed || selectedDirectSessionId !== null ? null : (
        <p className="room-panel__read-only" role="status">
          {archived ? <Archive aria-hidden="true" /> : <MessageSquareOff aria-hidden="true" />}
          {t(archived ? 'roomWorkspace.archivedNotice' : 'roomWorkspace.readOnlyNotice')}
        </p>
      )}
      {publicConversationVisible && invitation !== null && principal !== null ? (
        <div className="room-first-reply">
          <strong>{invitation.displayName}</strong>
          <InviteReplyProgress
            gateway={messageGateway}
            agentId={invitation.agentId}
            roomId={invitation.roomId}
            startedAt={invitation.startedAt}
            principalId={principal.principalId}
          />
          <button
            type="button"
            className="ar-icon-button"
            aria-label={t('agentInvite.close')}
            onClick={() => {
              setInvitation(null);
            }}
          >
            <X aria-hidden="true" />
          </button>
        </div>
      ) : null}
      {publicConversationVisible && room.notice !== undefined ? (
        <RoomNoticeBar
          key={room.roomId}
          memberNames={room.memberNames}
          notice={room.notice}
          onEdit={
            noticeEditable && principal !== null
              ? () => {
                  setRoomSettings({ section: 'notice' });
                }
              : undefined
          }
          roomId={room.roomId}
          selfMatrixUserId={principal?.matrixUserId ?? null}
        />
      ) : null}
    </>
  );
  const conversationContent = (
    <div
      className={conversationLayout ? 'room-desk__content' : 'room-panel__content'}
      id="workspace-current-view"
      role="tabpanel"
      aria-labelledby={`workspace-tab-${panelView}`}
    >
      <div className="workspace-room-content" hidden={selectedDirectSessionId !== null}>
        <MessageLayer
          active={publicConversationVisible}
          focusedConversationMessageId={focusedConversationMessageId}
          participants={room.agents}
          {...(room.memberNames === undefined ? {} : { memberNames: room.memberNames })}
          catalogId={catalogId}
          onSelectedMessageChange={onSelectedMessageChange}
          roomId={room.roomId}
          roomName={room.name}
          privateRoom={room.encrypted === true}
          selectedMessageId={selectedDirectSessionId === null ? selectedMessageId : null}
          view={panelView}
          writesAllowed={writesAllowed}
        />
      </div>
      <DirectConversationDock
        activeCatalogId={selectedDirectSessionId}
        controller={directSessions}
        onSelectedMessageChange={onSelectedMessageChange}
        selectedMessageId={selectedMessageId}
        view={panelView}
      />
    </div>
  );
  const agentDrawer =
    drawer === null ? null : (
      <WorkspaceDrawer label={t('roomWorkspace.members')} variant={drawer} onClose={closeDrawer}>
        {rosterWith(true)}
      </WorkspaceDrawer>
    );
  const inspector = (
    <AnimatePresence>
      {selectedAgent === null ? null : (
        <AgentInspector
          hasBackgroundReception={
            roster.wakeableAgentIds.has(selectedAgent.agentId) ||
            reception.some(
              (view) =>
                view.state.agentId === selectedAgent.agentId &&
                view.state.binding.policy.roomId === room.roomId,
            )
          }
          wakeable={roster.wakeableAgentIds.has(selectedAgent.agentId)}
          receptionControls={
            principal !== null && localRuntime.isAvailable() ? (
              <ReceptionPanel agentId={selectedAgent.agentId} roomId={room.roomId} />
            ) : principal !== null ? (
              <ReceptionOwnershipPanel agentId={selectedAgent.agentId} roomId={room.roomId} />
            ) : undefined
          }
          actionFailure={inspectorFailure(directSessions)}
          agent={selectedAgent}
          blocked={directSessions.isBlocked(selectedAgent)}
          // 右栏在的时候详情盖在表格上，关掉就是回到表格；右栏收起来时它是从右边出来的一层。
          closeAs={conversationLayout && sideColumn ? 'back' : 'close'}
          observedAtUnixMs={room.observedAtUnixMs}
          key="agent-inspector"
          pendingAction={
            directSessions.opening ? 'message' : directSessions.blocking ? 'block' : null
          }
          // 屏蔽完详情留着：上面说已屏蔽，按钮换成解除屏蔽，点错了马上能改回来。
          onBlock={() => {
            void directSessions.setBlocked(selectedAgent, true);
          }}
          onUnblock={() => {
            void directSessions.setBlocked(selectedAgent, false);
          }}
          onClose={() => {
            if (conversationLayout) returnFocusFrom.current = selectedAgent.agentId;
            else spatial.current?.focus();
            onSelectedAgentChange(null);
          }}
          onMessage={(agentId) => {
            void directSessions.openAgent(agentId).then((result) => {
              if (result.ok) {
                onSelectedAgentChange(null);
                onSelectedDirectSessionChange(result.value.catalogId);
              }
            });
          }}
        />
      )}
    </AnimatePresence>
  );

  if (conversationLayout)
    return (
      <>
        <RoomDesk
          agentCount={roster.agentCount}
          inspector={inspector}
          inspecting={selectedAgent !== null}
          inviteAgent={
            archived
              ? null
              : {
                  open: inviteOpen,
                  onOpen: () => {
                    setInviteOpen(true);
                  },
                }
          }
          invitePeople={
            invitable === null
              ? null
              : {
                  open: peopleInviteOpen,
                  onOpen: () => {
                    setPeopleInviteOpen(true);
                  },
                }
          }
          kind={roomKind}
          layoutSwitch={layoutSwitch}
          liveSummary={t('lobby.liveSummary', { count: attendance.present, room: room.name })}
          mainRoomSelected={selectedDirectSessionId === null}
          mainRoomUnread={activity.unread}
          onOpenAgents={() => {
            setDrawer('members');
          }}
          onOpenSettings={
            principal === null
              ? null
              : () => {
                  setRoomSettings({});
                }
          }
          onSelectMainRoom={() => {
            openRoomPanel('conversation');
          }}
          pendingJoinRequests={pendingJoinRequests}
          peopleCount={roster.people.people.length}
          rail={directSessions.sessions.length === 0 ? null : directNavigation}
          roomName={room.name}
          share={
            shareRoom === null
              ? null
              : {
                  open: shareOpen,
                  onOpen: () => {
                    setShareOpen(true);
                  },
                }
          }
          side={rosterWith(false)}
          view={panelView}
        >
          {conversationNotices}
          <WorkspaceViewTabs
            allowSpace={false}
            // 私聊里的“对话”是那场私聊，主房间的新消息标在回主房间的地方。
            conversationUnread={selectedDirectSessionId === null ? activity.unread : 0}
            onChange={onViewChange}
            value={panelView}
          />
          {conversationContent}
        </RoomDesk>
        {dialogs}
        {agentDrawer}
      </>
    );

  return (
    <main className="lobby-workspace lobby-game" id="main-content" data-view={activeView}>
      <p aria-atomic="true" aria-live="polite" className="sr-only">
        {t('lobby.liveSummary', { count: attendance.present, room: room.name })}
      </p>
      <div className="room-scene">
        <LobbySpatialView
          room={room}
          projection={projection}
          speech={speech}
          onOpenSpeech={openSpeech}
          onSelectHuman={() => {
            openRoomPanel('conversation');
          }}
          selectedAgentId={selectedAgentId}
          onSelectAgent={selectAgent}
          ref={spatial}
        />
      </div>
      <RoomBeacon
        agentCount={attendance.present}
        {...roomKind}
        layoutSwitch={layoutSwitch}
        membersButtonRef={membersButton}
        roomName={room.name}
        onOpenSettings={
          principal === null
            ? null
            : () => {
                setRoomSettings({});
              }
        }
        onOpenMembers={() => {
          setDrawer('members');
        }}
        pendingJoinRequests={pendingJoinRequests}
        {...(room.topic === undefined ? {} : { topic: room.topic })}
      />
      <p className="room-scene-hint">{t('roomGame.hint')}</p>
      {attendance.present === 0 && !archived ? (
        <div className="room-empty-presence" role="status">
          <strong>
            {t(attendance.reconnecting > 0 ? 'studio.reconnectingRoom' : 'studio.emptyRoom')}
          </strong>
          <p>{t('studio.emptyHint')}</p>
          <div className="room-empty-presence__actions">
            <button
              data-tone="primary"
              type="button"
              onClick={() => {
                setInviteOpen(true);
              }}
            >
              <Bot aria-hidden="true" />
              {t('studio.inviteAgent')}
            </button>
            <button
              type="button"
              onClick={() => {
                setDrawer('members');
              }}
            >
              {t('studio.findAway')}
            </button>
          </div>
        </div>
      ) : null}
      <div className="room-human-presence" role="group" aria-label={t('roomGame.people')}>
        {(projection.humans ?? []).slice(0, 3).map((human) => (
          <button
            type="button"
            key={human.matrixUserId}
            onClick={() => {
              openRoomPanel('conversation');
            }}
            aria-label={t(human.isSelf ? 'roomGame.selfCharacter' : 'roomGame.humanCharacter', {
              name: human.displayName,
            })}
          >
            <UsersRound aria-hidden="true" />
            <span>{human.isSelf ? t('roomGame.self') : human.displayName}</span>
          </button>
        ))}
      </div>
      <nav className="room-toolbelt" aria-label={t('roomGame.actions')}>
        <button
          type="button"
          aria-pressed={activeView === 'conversation' && selectedDirectSessionId === null}
          onClick={() => {
            openRoomPanel('conversation');
          }}
        >
          <MessageCircle aria-hidden="true" />
          <span className="room-toolbelt__label">{t('roomGame.chat')}</span>
          <span className="room-toolbelt__short" aria-hidden="true">
            {t('roomGame.chatShort')}
          </span>
          {activity.unread === 0 ? null : (
            <span
              className="room-unread"
              aria-label={t('roomGame.unread', { count: activity.unread })}
            >
              {activity.unread > 99 ? '99+' : activity.unread}
            </span>
          )}
        </button>
        <button
          type="button"
          aria-pressed={activeView === 'resources' && selectedDirectSessionId === null}
          onClick={() => {
            openRoomPanel('resources');
          }}
        >
          <Files aria-hidden="true" />
          <span className="room-toolbelt__label">{t('roomGame.resources')}</span>
          <span className="room-toolbelt__short" aria-hidden="true">
            {t('roomGame.resourcesShort')}
          </span>
        </button>
        <button
          type="button"
          onClick={() => {
            setDrawer('members');
          }}
        >
          <UsersRound aria-hidden="true" />
          <span className="room-toolbelt__label">{t('roomGame.characters')}</span>
          <span className="room-toolbelt__short" aria-hidden="true">
            {t('roomGame.charactersShort')}
          </span>
        </button>
        {archived ? null : (
          <button
            type="button"
            aria-haspopup="dialog"
            aria-expanded={inviteOpen}
            onClick={() => {
              setInviteOpen(true);
            }}
          >
            <Bot aria-hidden="true" />
            <span className="room-toolbelt__label">{t('agentInvite.open')}</span>
            <span className="room-toolbelt__short" aria-hidden="true">
              {t('roomGame.inviteShort')}
            </span>
          </button>
        )}
        {invitable === null ? null : (
          <button
            type="button"
            aria-haspopup="dialog"
            aria-expanded={peopleInviteOpen}
            onClick={() => {
              setPeopleInviteOpen(true);
            }}
          >
            <UserPlus aria-hidden="true" />
            <span className="room-toolbelt__label">{t('privateRooms.inviteLink.open')}</span>
            <span className="room-toolbelt__short" aria-hidden="true">
              {t('privateRooms.inviteLink.openShort')}
            </span>
          </button>
        )}
        {shareRoom === null ? null : (
          <button
            type="button"
            aria-haspopup="dialog"
            aria-expanded={shareOpen}
            onClick={() => {
              setShareOpen(true);
            }}
          >
            <Share2 aria-hidden="true" />
            <span className="room-toolbelt__label">{t('roomShare.open')}</span>
            <span className="room-toolbelt__short" aria-hidden="true">
              {t('roomShare.open')}
            </span>
          </button>
        )}
      </nav>
      {dialogs}
      <section
        className="room-panel"
        data-size={panelSize}
        hidden={activeView === 'space'}
        aria-label={t('roomGame.panel')}
      >
        <header className="room-panel__header">
          <WorkspaceViewTabs value={panelView} onChange={onViewChange} allowSpace={false} />
          <div className="room-panel__header-actions">
            <button
              type="button"
              className="ar-icon-button room-panel__widen"
              aria-label={t(panelSize === 'wide' ? 'roomGame.narrowPanel' : 'roomGame.widenPanel')}
              title={t(panelSize === 'wide' ? 'roomGame.narrowPanel' : 'roomGame.widenPanel')}
              onClick={() => {
                const next = panelSize === 'wide' ? 'regular' : 'wide';
                setPanelSize(next);
                writePanelSize(next);
              }}
            >
              {panelSize === 'wide' ? (
                <Minimize2 aria-hidden="true" />
              ) : (
                <Maximize2 aria-hidden="true" />
              )}
            </button>
            <button
              type="button"
              className="ar-icon-button"
              aria-label={t('roomGame.closePanel')}
              onClick={() => {
                onViewChange('space');
                spatial.current?.focus();
              }}
            >
              <X aria-hidden="true" />
            </button>
          </div>
        </header>
        {conversationNotices}
        {conversationContent}
      </section>
      {agentDrawer}
      {inspector}
    </main>
  );
}

/** 详情里只说屏蔽、解除屏蔽、私聊没做成；读私聊列表出的错在私聊那边说。 */
function inspectorFailure(controller: DirectSessionController): AgentInspectorFailure | null {
  if (controller.failure === null || controller.failedAction === null) return null;
  return {
    action: controller.failedAction === 'open' ? 'message' : controller.failedAction,
    code: controller.failure.code,
  };
}

/**
 * 能在这个房间里邀请人时是这个私人房间：房主，或者有“邀请”权限的已加入成员；归档了的不行
 * （`specs/room-people/design.md`）。没有邀请链接服务（测试、夹具）时也没有。
 */
function invitableRoom(
  rooms: readonly PrivateRoom[],
  catalogId: string,
  principalId: string | null,
): PrivateRoom | null {
  if (principalId === null) return null;
  const room = rooms.find((entry) => entry.catalogId === catalogId);
  if (room?.status !== 'active') return null;
  return room.ownerPrincipalId === principalId || allows(memberFor(room, principalId), 'invite')
    ? room
    : null;
}

/** 你在这个私人房间里带进来的 Agent 能不能说话：房主都能，成员要有“自动发言”。不是私人房间的都能。 */
/** 你是这间房的房主，或者有“管理”。公共大厅和你不在里面的房间都不算。 */
function managesRoom(
  rooms: readonly PrivateRoom[],
  catalogId: string,
  principalId: string | null,
): boolean {
  const room = rooms.find((entry) => entry.catalogId === catalogId);
  if (room === undefined || principalId === null) return false;
  return room.ownerPrincipalId === principalId || allows(memberFor(room, principalId), 'manage');
}

function agentsSpeakIn(
  rooms: readonly PrivateRoom[],
  catalogId: string,
  principalId: string | null,
): boolean {
  const room = rooms.find((entry) => entry.catalogId === catalogId);
  if (room === undefined || principalId === null || room.ownerPrincipalId === principalId) {
    return true;
  }
  return allows(memberFor(room, principalId), 'automate');
}
