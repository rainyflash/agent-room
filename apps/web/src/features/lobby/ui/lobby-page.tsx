import { Button } from '@agent-room/ui-system';
import { RotateCw, ShieldCheck } from 'lucide-react';
import { AnimatePresence } from 'motion/react';
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { useTranslation } from 'react-i18next';

import { useAppServices } from '@/app/app-services';
import { AutomationGrantHub } from '@/features/automation/ui/automation-grant-hub';
import type { DirectAgent } from '@/features/direct-sessions/domain/direct-session';
import { DirectConversationDock } from '@/features/direct-sessions/ui/direct-conversation-dock';
import { useDirectSessionController } from '@/features/direct-sessions/ui/use-direct-session-controller';
import { LobbyRoomStore } from '@/features/lobby/application/lobby-room-store';
import type { LobbyRoom } from '@/features/lobby/domain/lobby';
import { projectLobbyScene } from '@/features/lobby/domain/scene-projection';
import type { LobbySceneLabels } from '@/features/lobby/scene/lobby-scene';
import { AgentInspector } from '@/features/lobby/ui/agent-inspector';
import { ListModeRoster, type ListModeRosterHandle } from '@/features/lobby/ui/list-mode-roster';
import { LobbySceneSurface } from '@/features/lobby/ui/lobby-scene-surface';
import type { LobbySceneSurfaceHandle } from '@/features/lobby/ui/lobby-scene-surface';
import { LobbyStateBoundary } from '@/features/lobby/ui/lobby-state-boundary';
import { RoomBeacon } from '@/features/lobby/ui/room-beacon';
import { SceneSemanticRoster } from '@/features/lobby/ui/scene-semantic-roster';
import { SignalDock, type LobbyViewMode } from '@/features/lobby/ui/signal-dock';
import { useCompactLobby } from '@/features/lobby/ui/use-compact-lobby';
import { MessageLayer } from '@/features/messages/ui/message-layer';
import { PrivateRoomHub } from '@/features/private-rooms/ui/private-room-hub';
import { useAccountPreferences } from '@/features/preferences/ui/account-preferences-provider';
import type { WebSession } from '@/features/session/domain/session';

export type LobbyPageProps = {
  readonly catalogId: string;
  readonly onEnterRoom: (catalogId: string, matrixRoomId: string) => void;
  readonly onExitRoom: () => void;
  readonly onOpenSecurity: () => void;
  readonly onSelectedAgentChange: (agentId: string | null) => void;
  readonly onSelectedDirectSessionChange: (catalogId: string | null) => void;
  readonly onSelectedMessageChange: (messageId: string | null) => void;
  readonly principal: WebSession | null;
  readonly roomId: string;
  readonly selectedAgentId: string | null;
  readonly selectedDirectSessionId: string | null;
  readonly selectedMessageId: string | null;
};

export function LobbyPage({
  catalogId,
  onEnterRoom,
  onExitRoom,
  onOpenSecurity,
  onSelectedAgentChange,
  onSelectedDirectSessionChange,
  onSelectedMessageChange,
  principal,
  roomId,
  selectedAgentId,
  selectedDirectSessionId,
  selectedMessageId,
}: LobbyPageProps) {
  const { lobby } = useAppServices();
  const roomStore = useMemo(() => new LobbyRoomStore(lobby, roomId), [lobby, roomId]);
  const state = useSyncExternalStore(
    roomStore.subscribe,
    roomStore.getSnapshot,
    roomStore.getSnapshot,
  );

  if (state.kind !== 'ready') {
    return <LobbyStateBoundary onRetry={roomStore.retry} state={state} />;
  }

  return (
    <ReadyLobby
      catalogId={catalogId}
      key={state.room.roomId}
      onEnterRoom={onEnterRoom}
      onExitRoom={onExitRoom}
      onOpenSecurity={onOpenSecurity}
      onSelectedAgentChange={onSelectedAgentChange}
      onSelectedDirectSessionChange={onSelectedDirectSessionChange}
      onSelectedMessageChange={onSelectedMessageChange}
      principal={principal}
      room={state.room}
      selectedAgentId={selectedAgentId}
      selectedDirectSessionId={selectedDirectSessionId}
      selectedMessageId={selectedMessageId}
    />
  );
}

type ReadyLobbyProps = Omit<LobbyPageProps, 'roomId'> & {
  readonly room: LobbyRoom;
};

function ReadyLobby({
  catalogId,
  onEnterRoom,
  onExitRoom,
  onOpenSecurity,
  onSelectedAgentChange,
  onSelectedDirectSessionChange,
  onSelectedMessageChange,
  principal,
  room,
  selectedAgentId,
  selectedDirectSessionId,
  selectedMessageId,
}: ReadyLobbyProps) {
  const { i18n, t } = useTranslation();
  const { accessManagement, automation, controlPlane } = useAppServices();
  const accountPreferences = useAccountPreferences();
  const compact = useCompactLobby();
  const listRef = useRef<ListModeRosterHandle>(null);
  const sceneRef = useRef<LobbySceneSurfaceHandle>(null);
  const [sceneAvailable, setSceneAvailable] = useState(true);
  const [zoom, setZoom] = useState(1);
  const directSessions = useDirectSessionController(principal !== null);
  const projection = useMemo(
    () => projectLobbyScene(room, selectedAgentId),
    [room, selectedAgentId],
  );
  const preferredMode = accountPreferences.snapshot.values.lobbyView;
  const mode: LobbyViewMode = compact || !sceneAvailable ? 'list' : preferredMode;
  const selectedAgent =
    projection.nodes.find((agent) => agent.agentId === projection.selectedAgentId) ?? null;
  const languageKey = i18n.resolvedLanguage ?? i18n.language;
  const labels = useMemo<LobbySceneLabels>(
    () => ({
      agentAccessibilityLabel: (agent) =>
        t('lobby.agent.accessibility', {
          name: agent.displayName,
          status: t(`lobby.status.${agent.status}`),
        }),
      canvas: t('lobby.scene.canvasLabel'),
      zones: {
        active: t('lobby.zone.active'),
        attention: t('lobby.zone.attention'),
        available: t('lobby.zone.available'),
      },
    }),
    [languageKey, t],
  );

  useEffect(() => {
    if (selectedAgentId !== null && projection.selectedAgentId === null) {
      onSelectedAgentChange(null);
    }
  }, [onSelectedAgentChange, projection.selectedAgentId, selectedAgentId]);

  const restoreSelectionFocus = (): void => {
    if (mode === 'scene') {
      sceneRef.current?.focus();
    } else {
      listRef.current?.focusSelected();
    }
  };

  return (
    <main className="lobby-shell" id="main-content">
      <p aria-atomic="true" aria-live="polite" className="sr-only">
        {t('lobby.liveSummary', {
          count: room.agents.length,
          room: room.name,
        })}
      </p>
      <RoomBeacon
        actions={
          principal === null ? undefined : (
            <>
              <AutomationGrantHub
                accessManagement={accessManagement}
                automation={automation}
                catalogId={catalogId}
                onReauthenticate={() => {
                  controlPlane.beginAuthentication(
                    `${window.location.pathname}${window.location.search}${window.location.hash}`,
                  );
                }}
                recentlyAuthenticated={principal.recentlyAuthenticated}
                roomName={room.name}
              />
              <Button
                aria-label={t('security.launcher')}
                className="security-launcher"
                icon={<ShieldCheck aria-hidden="true" />}
                onClick={onOpenSecurity}
                size="compact"
                tone="quiet"
              >
                {t('security.launcher')}
              </Button>
              <PrivateRoomHub
                currentCatalogId={catalogId}
                onEnterRoom={onEnterRoom}
                onExitRoom={onExitRoom}
                principal={principal}
              />
            </>
          )
        }
        agentCount={room.agents.length}
        catalogId={catalogId}
        roomName={room.name}
        {...(room.topic === undefined ? {} : { topic: room.topic })}
      />
      <div className={`lobby-stage lobby-stage--${mode}`}>
        {mode === 'scene' ? (
          <>
            <LobbySceneSurface
              labels={labels}
              languageKey={languageKey}
              onFailure={() => {
                setSceneAvailable(false);
              }}
              onSelectAgent={onSelectedAgentChange}
              onZoomChange={setZoom}
              projection={projection}
              ref={sceneRef}
            />
            <SceneSemanticRoster
              nodes={projection.nodes}
              selectedAgentId={projection.selectedAgentId}
            />
            {projection.nodes.length === 0 ? <EmptyLobby /> : null}
          </>
        ) : (
          <>
            {!sceneAvailable && !compact ? (
              <div className="scene-fallback" role="status">
                <span>{t('lobby.scene.failed')}</span>
                <Button
                  icon={<RotateCw aria-hidden="true" />}
                  onClick={() => {
                    setSceneAvailable(true);
                    accountPreferences.setLobbyView('scene');
                  }}
                  size="compact"
                  tone="ghost"
                >
                  {t('lobby.scene.retry')}
                </Button>
              </div>
            ) : null}
            <ListModeRoster
              agents={room.agents}
              onSelectAgent={onSelectedAgentChange}
              ref={listRef}
              selectedAgentId={projection.selectedAgentId}
            />
          </>
        )}
      </div>
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
                if (result.ok) {
                  onSelectedAgentChange(null);
                }
              });
            }}
            onClose={() => {
              restoreSelectionFocus();
              onSelectedAgentChange(null);
            }}
            onMessage={(agentId) => {
              void directSessions.openAgent(agentId).then((result) => {
                if (!result.ok) {
                  return;
                }
                onSelectedAgentChange(null);
                onSelectedDirectSessionChange(result.value.catalogId);
              });
            }}
          />
        )}
      </AnimatePresence>
      {selectedDirectSessionId === null ? (
        <MessageLayer
          onSelectedMessageChange={onSelectedMessageChange}
          roomId={room.roomId}
          roomName={room.name}
          selectedMessageId={selectedMessageId}
        />
      ) : null}
      <DirectConversationDock
        activeCatalogId={selectedDirectSessionId}
        controller={directSessions}
        onActiveSessionChange={onSelectedDirectSessionChange}
        onSelectedMessageChange={onSelectedMessageChange}
        selectedMessageId={selectedMessageId}
      />
      {compact || selectedDirectSessionId !== null ? null : (
        <SignalDock
          mode={mode}
          onModeChange={(nextMode) => {
            if (nextMode === 'scene' && !sceneAvailable) {
              return;
            }
            accountPreferences.setLobbyView(nextMode);
          }}
          onResetViewport={() => {
            sceneRef.current?.resetViewport();
          }}
          onZoomBy={(factor) => {
            sceneRef.current?.zoomBy(factor);
          }}
          sceneAvailable={sceneAvailable}
          zoom={zoom}
        />
      )}
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

function EmptyLobby() {
  const { t } = useTranslation();
  return (
    <section aria-labelledby="empty-lobby-title" className="empty-lobby">
      <p className="eyebrow">{t('lobby.empty.eyebrow')}</p>
      <h1 id="empty-lobby-title">{t('lobby.empty.title')}</h1>
      <p>{t('lobby.empty.detail')}</p>
    </section>
  );
}
