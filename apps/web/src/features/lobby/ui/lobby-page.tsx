import { Files, MessageCircle, UsersRound, X } from 'lucide-react';
import { AnimatePresence } from 'motion/react';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';
import { useAppServices } from '@/app/app-services';
import { DesktopRuntimeSurface } from '@/features/desktop/ui/desktop-runtime-surface';
import type { DirectAgent } from '@/features/direct-sessions/domain/direct-session';
import { DirectConversationDock } from '@/features/direct-sessions/ui/direct-conversation-dock';
import { useDirectSessionController } from '@/features/direct-sessions/ui/use-direct-session-controller';
import { LobbyExperienceStore } from '@/features/lobby/application/lobby-experience-store';
import { RoomActivityStore } from '@/features/lobby/application/room-activity-store';
import {
  RoomMessagesProvider,
  useRoomMessages,
} from '@/features/messages/ui/room-messages-context';
import { projectRoomSpeech } from '@/features/lobby/domain/room-speech';
import type { LobbyRoom } from '@/features/lobby/domain/lobby';
import type { RoomWorkspaceView } from '@/features/lobby/domain/workspace-view';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';
import { AgentInspector } from '@/features/lobby/ui/agent-inspector';
import { ListModeRoster } from '@/features/lobby/ui/list-mode-roster';
import { LobbyRoomActions } from '@/features/lobby/ui/lobby-room-actions';
import {
  LobbySpatialView,
  type LobbySpatialViewHandle,
} from '@/features/lobby/ui/lobby-spatial-view';
import { LobbyStateBoundary } from '@/features/lobby/ui/lobby-state-boundary';
import { RoomBeacon } from '@/features/lobby/ui/room-beacon';
import { WorkspaceDrawer } from '@/features/lobby/ui/workspace-drawer';
import { WorkspaceNavigation } from '@/features/lobby/ui/workspace-navigation';
import { WorkspaceViewTabs } from '@/features/lobby/ui/workspace-view-tabs';
import { MessageLayer } from '@/features/messages/ui/message-layer';
import type { WebSession } from '@/features/session/domain/session';
import './lobby-workspace.css';
import './lobby-game.css';

export type LobbyPageProps = {
  readonly catalogId: string;
  readonly onEnterRoom: (catalogId: string, matrixRoomId: string) => void;
  readonly onExitRoom: () => void;
  readonly onOpenSecurity: () => void;
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
  const { lobby, messages } = useAppServices();
  const store = useMemo(
    () => new LobbyExperienceStore(lobby, messages, props.roomId, props.principal),
    [lobby, messages, props.roomId, props.principal?.matrixUserId, props.principal?.displayName],
  );
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  if (state.kind !== 'ready')
    return (
      <>
        <LobbyStateBoundary onRetry={store.retry} state={state} />
        <DesktopRuntimeSurface />
      </>
    );
  return (
    <RoomMessagesProvider store={store.messages}>
      <ReadyLobby {...props} key={state.room.roomId} room={state.room} scene={state.projection} />
    </RoomMessagesProvider>
  );
}

function ReadyLobby({
  catalogId,
  onEnterRoom,
  onExitRoom,
  onOpenSecurity,
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
  const directSessions = useDirectSessionController(principal !== null);
  const [drawer, setDrawer] = useState<'navigation' | 'members' | null>(null);
  const membersButton = useRef<HTMLButtonElement>(null);
  const spatial = useRef<LobbySpatialViewHandle>(null);
  const projection = useMemo(
    () => ({
      ...scene,
      selectedAgentId: room.agents.some((agent) => agent.agentId === selectedAgentId)
        ? selectedAgentId
        : null,
    }),
    [scene, room.agents, selectedAgentId],
  );
  const selectedAgent =
    projection.nodes.find((agent) => agent.agentId === projection.selectedAgentId) ?? null;
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
    if (selectedAgentId !== null && projection.selectedAgentId === null)
      onSelectedAgentChange(null);
  }, [onSelectedAgentChange, projection.selectedAgentId, selectedAgentId]);

  const closeDrawer = (): void => {
    setDrawer(null);
  };
  const selectAgent = (id: string | null): void => {
    closeDrawer();
    onSelectedAgentChange(id);
  };
  const navigation = (
    <WorkspaceNavigation
      activeDirectId={selectedDirectSessionId}
      controller={directSessions}
      onActivateRoom={() => {
        closeDrawer();
        onViewChange('space');
      }}
      onActivateDirect={(id) => {
        closeDrawer();
        onSelectedDirectSessionChange(id);
      }}
      roomName={room.name}
      userName={principal?.displayName ?? null}
      actions={
        principal === null ? null : (
          <LobbyRoomActions
            catalogId={catalogId}
            roomName={room.name}
            principal={principal}
            onEnterRoom={onEnterRoom}
            onExitRoom={onExitRoom}
            onOpenSecurity={onOpenSecurity}
          />
        )
      }
    />
  );
  const roster = (
    <div className="workspace-members">
      <ListModeRoster
        agents={room.agents}
        onSelectAgent={selectAgent}
        selectedAgentId={selectedAgentId}
        variant="compact"
      />
      <p className="workspace-members__note">{t('roomWorkspace.memberNote')}</p>
    </div>
  );

  return (
    <main className="lobby-workspace lobby-game" id="main-content" data-view={activeView}>
      <p aria-atomic="true" aria-live="polite" className="sr-only">
        {t('lobby.liveSummary', { count: room.agents.length, room: room.name })}
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
        agentCount={room.agents.length}
        membersButtonRef={membersButton}
        roomName={room.name}
        onOpenNavigation={() => {
          setDrawer('navigation');
        }}
        onOpenMembers={() => {
          setDrawer('members');
        }}
        {...(room.topic === undefined ? {} : { topic: room.topic })}
      />
      <p className="room-scene-hint">{t('roomGame.hint')}</p>
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
      <DesktopRuntimeSurface placement="game" />
      <nav className="room-toolbelt" aria-label={t('roomGame.actions')}>
        <button
          type="button"
          aria-pressed={activeView === 'conversation' && selectedDirectSessionId === null}
          onClick={() => {
            openRoomPanel('conversation');
          }}
        >
          <MessageCircle aria-hidden="true" />
          {t('roomGame.chat')}
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
          {t('roomGame.resources')}
        </button>
        <button
          type="button"
          onClick={() => {
            setDrawer('members');
          }}
        >
          <UsersRound aria-hidden="true" />
          {t('roomGame.characters')}
        </button>
      </nav>
      <section
        className="room-panel"
        hidden={activeView === 'space'}
        aria-label={t('roomGame.panel')}
      >
        <header className="room-panel__header">
          <WorkspaceViewTabs value={panelView} onChange={onViewChange} allowSpace={false} />
          <button
            type="button"
            aria-label={t('roomGame.closePanel')}
            onClick={() => {
              onViewChange('space');
              spatial.current?.focus();
            }}
          >
            <X aria-hidden="true" />
          </button>
        </header>
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
              catalogId={catalogId}
              onSelectedMessageChange={onSelectedMessageChange}
              roomId={room.roomId}
              roomName={room.name}
              selectedMessageId={selectedDirectSessionId === null ? selectedMessageId : null}
              view={panelView}
            />
          </div>
          <DirectConversationDock
            activeCatalogId={selectedDirectSessionId}
            controller={directSessions}
            onActiveSessionChange={onSelectedDirectSessionChange}
            onSelectedMessageChange={onSelectedMessageChange}
            selectedMessageId={selectedMessageId}
            view={panelView}
          />
        </div>
      </section>
      {drawer === null ? null : (
        <WorkspaceDrawer
          label={t(drawer === 'navigation' ? 'roomWorkspace.navigation' : 'roomWorkspace.members')}
          variant={drawer}
          onClose={closeDrawer}
        >
          {drawer === 'navigation' ? navigation : roster}
        </WorkspaceDrawer>
      )}
      <AnimatePresence>
        {selectedAgent === null ? null : (
          <AgentInspector
            actionFailure={directSessions.failure?.code ?? null}
            agent={selectedAgent}
            key={selectedAgent.agentId}
            pendingAction={
              directSessions.opening ? 'message' : directSessions.blocking ? 'block' : null
            }
            onBlock={() => {
              void directSessions.setBlocked(toDirectAgent(selectedAgent), true).then((result) => {
                if (result.ok) onSelectedAgentChange(null);
              });
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

function toDirectAgent(agent: LobbyRoom['agents'][number]): DirectAgent {
  return Object.freeze({
    agentId: agent.agentId,
    avatarContentId: null,
    displayName: agent.displayName,
    matrixUserId: agent.matrixUserId,
  });
}
