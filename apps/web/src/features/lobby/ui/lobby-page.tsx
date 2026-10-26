import { usePublishedDownload } from '@/features/updates/ui/use-published-download';
import {
  Archive,
  Bot,
  Files,
  Maximize2,
  MessageCircle,
  MessageSquareOff,
  Minimize2,
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
import type { RoomWorkspaceView } from '@/features/lobby/domain/workspace-view';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';
import { AgentInspector, type AgentInspectorFailure } from '@/features/lobby/ui/agent-inspector';
import { AgentRosterPolicyPanel } from './agent-roster-policy-panel';
import { ListModeRoster } from '@/features/lobby/ui/list-mode-roster';
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
import { RoomBeacon } from '@/features/lobby/ui/room-beacon';
import { WorkspaceDrawer } from '@/features/lobby/ui/workspace-drawer';
import { DirectSessionNavigation } from '@/features/direct-sessions/ui/direct-session-navigation';
import { WorkspaceViewTabs } from '@/features/lobby/ui/workspace-view-tabs';
import { MessageLayer } from '@/features/messages/ui/message-layer';
import {
  usePrivateRoomList,
  usePrivateRoomOrigin,
} from '@/features/private-rooms/data/private-room-queries';
import type { WebSession } from '@/features/session/domain/session';
import './lobby-workspace.css';
import './lobby-game.css';

// 对话面板放大与否只影响这台设备的布局，存在本机；读写失败（隐私模式等）就只在本次有效。
const panelSizeKey = 'agent-room.room-panel-size';
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
  readonly onExitRoom: () => void;
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
  onExitRoom,
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
    privateRooms,
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
  const [invitation, setInvitation] = useState<ConnectedInvitation | null>(null);
  const [panelSize, setPanelSize] = useState<PanelSize>(readPanelSize);
  const owner =
    principal === null
      ? null
      : { principalId: principal.principalId, displayName: principal.displayName };
  const membersButton = useRef<HTMLButtonElement>(null);
  const spatial = useRef<LobbySpatialViewHandle>(null);
  const origin = usePrivateRoomOrigin(
    privateRooms,
    catalogId,
    principal !== null && room.encrypted === true,
  );
  // 归档了的私人房间照样打得开、看得到之前的消息，只是谁都不能再说话、也不能再接入 Agent。
  const privateRoomList = usePrivateRoomList(
    privateRooms,
    principal?.principalId,
    principal !== null && room.encrypted === true,
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
  // 能不能发消息看 Matrix 的发言门槛：归档、被禁言、只能查看时输入框都关着，免得发了才说失败。
  const writesAllowed = !archived && room.canSpeak !== false;
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
  const activeView = selectedDirectSessionId !== null && view === 'space' ? 'conversation' : view;
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
  const selectAgent = (id: string | null): void => {
    closeDrawer();
    onSelectedAgentChange(id);
  };
  const roster = (
    <div className="workspace-members">
      <ListModeRoster
        agents={room.agents}
        observedAtUnixMs={room.observedAtUnixMs}
        onSelectAgent={selectAgent}
        selectedAgentId={selectedAgentId}
        variant="compact"
      />
      <p className="workspace-members__note">{t('roomWorkspace.memberNote')}</p>
      {principal === null ? null : (
        <DirectSessionNavigation
          activeCatalogId={selectedDirectSessionId}
          controller={directSessions}
          onActivate={(id) => {
            closeDrawer();
            onSelectedDirectSessionChange(id);
          }}
        />
      )}
      <AgentRosterPolicyPanel
        key={`${catalogId}:${String(room.archiveAfterDays ?? 7)}`}
        catalogId={catalogId}
        days={room.archiveAfterDays ?? 7}
      />
    </div>
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
        privateRoom={room.encrypted === true}
        archived={archived}
        serverReadable={origin.data?.ok === true && origin.data.value.ownerIsNetworkAgent}
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
      </nav>
      {roomSettings === null || principal === null ? null : (
        <RoomSettingsDialog
          catalogId={catalogId}
          initialSection={roomSettings.section}
          onClose={() => {
            setRoomSettings(null);
          }}
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
        <div
          className="room-panel__content"
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
      </section>
      {drawer === null ? null : (
        <WorkspaceDrawer label={t('roomWorkspace.members')} variant={drawer} onClose={closeDrawer}>
          {roster}
        </WorkspaceDrawer>
      )}
      <AnimatePresence>
        {selectedAgent === null ? null : (
          <AgentInspector
            hasBackgroundReception={reception.some(
              (view) =>
                view.state.agentId === selectedAgent.agentId &&
                view.state.binding.policy.roomId === room.roomId,
            )}
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
              spatial.current?.focus();
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
