import { Button } from '@agent-room/ui-system';
import { Clock3, RefreshCw, ShieldCheck, X } from 'lucide-react';
import { useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';

import { useRuntimeServices } from '@/app/app-services';
import { DesktopLobbyStore } from '@/features/desktop/application/desktop-lobby-store';
import { projectDesktopLobby } from '@/features/desktop/domain/desktop-lobby-projection';
import type { DesktopRuntimeGateway } from '@/features/desktop/domain/desktop-runtime';
import { DesktopConnectionPage } from '@/features/desktop/ui/desktop-connection-page';
import { useDesktopRuntimeController } from '@/features/desktop/ui/desktop-runtime-provider';
import type { LobbyAgent } from '@/features/lobby/domain/lobby';
import { projectLobbyScene } from '@/features/lobby/domain/scene-projection';
import type { LobbySceneLabels } from '@/features/lobby/scene/lobby-scene';
import { ListModeRoster } from '@/features/lobby/ui/list-mode-roster';
import {
  LobbySceneSurface,
  type LobbySceneSurfaceHandle,
} from '@/features/lobby/ui/lobby-scene-surface';
import { RoomBeacon } from '@/features/lobby/ui/room-beacon';
import { SignalDock, type LobbyViewMode } from '@/features/lobby/ui/signal-dock';
import { useListModeRequirement } from '@/features/lobby/ui/use-list-mode-requirement';

import './desktop-lobby-page.css';

export function DesktopLobbyPage() {
  const { t } = useTranslation();
  const services = useRuntimeServices();
  const runtime = useDesktopRuntimeController();
  if (services.runtimeMode !== 'desktop') {
    throw new Error('DesktopLobbyPage requires desktop runtime services.');
  }
  const target = runtime.snapshot?.agentTarget ?? null;
  const session = runtime.snapshot?.bridge.session ?? null;
  const ready = runtime.snapshot?.bridge.lifecycle.phase === 'ready';
  if (!ready || target === null || session === null || target.agentId !== session.agentId) {
    return <DesktopConnectionPage />;
  }
  return (
    <ReadyDesktopLobby
      catalogId={target.publicLobbyCatalogId}
      expectedRoomId={session.matrixRoomId}
      gateway={services.desktop}
      roomName={t('desktop.lobby.roomName')}
      topic={t('desktop.lobby.topic')}
    />
  );
}

type ReadyDesktopLobbyProps = {
  readonly catalogId: string;
  readonly expectedRoomId: string;
  readonly gateway: DesktopRuntimeGateway;
  readonly roomName: string;
  readonly topic: string;
};

function ReadyDesktopLobby({
  catalogId,
  expectedRoomId,
  gateway,
  roomName,
  topic,
}: ReadyDesktopLobbyProps) {
  const { i18n, t } = useTranslation();
  const store = useMemo(() => new DesktopLobbyStore(gateway), [gateway]);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [preferredMode, setPreferredMode] = useState<LobbyViewMode>('scene');
  const [zoom, setZoom] = useState(1);
  const sceneRef = useRef<LobbySceneSurfaceHandle>(null);
  const listModeRequirement = useListModeRequirement();

  if (state.kind === 'loading') {
    return <DesktopLobbyBoundary detail={t('desktop.lobby.loading')} />;
  }
  if (state.kind === 'failed') {
    return (
      <DesktopLobbyBoundary
        action={
          <Button icon={<RefreshCw aria-hidden="true" />} onClick={store.retry} tone="alert">
            {t('desktop.lobby.retry')}
          </Button>
        }
        detail={state.failure.code}
        title={t('desktop.lobby.failed')}
      />
    );
  }
  if (state.snapshot.identity.roomId !== expectedRoomId) {
    return (
      <DesktopLobbyBoundary
        action={
          <Button icon={<RefreshCw aria-hidden="true" />} onClick={store.retry} tone="alert">
            {t('desktop.lobby.retry')}
          </Button>
        }
        detail="desktop.lobby.room_mismatch"
        title={t('desktop.lobby.failed')}
      />
    );
  }

  const projected = projectDesktopLobby(state.snapshot, roomName, topic);
  const scene = projectLobbyScene(projected.room, selectedAgentId);
  const selectedAgent =
    projected.room.agents.find((agent) => agent.agentId === scene.selectedAgentId) ?? null;
  const mode: LobbyViewMode = listModeRequirement !== null ? 'list' : preferredMode;
  const labels: LobbySceneLabels = {
    canvas: t('lobby.scene.canvasLabel'),
    zones: {
      active: t('lobby.zone.active'),
      attention: t('lobby.zone.attention'),
      available: t('lobby.zone.available'),
    },
  };

  return (
    <main className="lobby-shell desktop-lobby" id="main-content">
      <RoomBeacon
        actions={
          <Button
            aria-label={t('desktop.lobby.refresh')}
            icon={<RefreshCw aria-hidden="true" />}
            onClick={store.retry}
            size="compact"
            tone="quiet"
          >
            <span className="desktop-lobby__refresh-label">{t('desktop.lobby.refresh')}</span>
          </Button>
        }
        agentCount={projected.room.agents.length}
        catalogId={catalogId}
        roomName={projected.room.name}
        {...(projected.room.topic === undefined ? {} : { topic: projected.room.topic })}
      />
      <div className={`lobby-stage lobby-stage--${mode}`}>
        {mode === 'scene' ? (
          <LobbySceneSurface
            labels={labels}
            languageKey={i18n.resolvedLanguage ?? i18n.language}
            onSelectAgent={setSelectedAgentId}
            onZoomChange={setZoom}
            projection={scene}
            ref={sceneRef}
          />
        ) : (
          <ListModeRoster
            agents={projected.room.agents}
            onSelectAgent={setSelectedAgentId}
            selectedAgentId={scene.selectedAgentId}
            selfAgentId={state.snapshot.identity.agent.agentId}
          />
        )}
      </div>
      <DesktopMessagePreviewRail messages={projected.messages} />
      {selectedAgent === null ? null : (
        <DesktopAgentPanel agent={selectedAgent} onClose={() => setSelectedAgentId(null)} />
      )}
      <SignalDock
        mode={mode}
        onModeChange={setPreferredMode}
        onResetViewport={() => sceneRef.current?.resetViewport()}
        onZoomBy={(factor) => sceneRef.current?.zoomBy(factor)}
        sceneAvailable={listModeRequirement === null}
        zoom={zoom}
      />
    </main>
  );
}

function DesktopLobbyBoundary({
  action,
  detail,
  title,
}: {
  readonly action?: ReactNode;
  readonly detail: string;
  readonly title?: string;
}) {
  const { t } = useTranslation();
  return (
    <main className="lobby-shell desktop-lobby desktop-lobby--boundary" id="main-content">
      <img alt="" src="/agent-room-mark.svg" />
      <p className="eyebrow">{t('desktop.lobby.eyebrow')}</p>
      <h1>{title ?? t('desktop.lobby.loading')}</h1>
      <code>{detail}</code>
      {action}
    </main>
  );
}

function DesktopMessagePreviewRail({
  messages,
}: {
  readonly messages: ReturnType<typeof projectDesktopLobby>['messages'];
}) {
  const { i18n, t } = useTranslation();
  return (
    <aside aria-label={t('desktop.lobby.messages')} className="desktop-message-rail">
      <header>
        <div>
          <p className="eyebrow">{t('desktop.lobby.signal')}</p>
          <h2>{t('desktop.lobby.messages')}</h2>
        </div>
        <span>{messages.length}</span>
      </header>
      {messages.length === 0 ? (
        <p className="desktop-message-rail__empty">{t('desktop.lobby.noMessages')}</p>
      ) : (
        <ol>
          {messages.map((message) => (
            <li key={message.messageId}>
              <div>
                <strong>{message.actor.agent.displayName}</strong>
                <time dateTime={new Date(message.createdAtUnixMs).toISOString()}>
                  <Clock3 aria-hidden="true" />
                  {new Intl.DateTimeFormat(i18n.resolvedLanguage ?? 'en', {
                    hour: '2-digit',
                    minute: '2-digit',
                  }).format(message.createdAtUnixMs)}
                </time>
              </div>
              <h3>{message.title}</h3>
              <p>{message.summary}</p>
            </li>
          ))}
        </ol>
      )}
    </aside>
  );
}

function DesktopAgentPanel({
  agent,
  onClose,
}: {
  readonly agent: LobbyAgent;
  readonly onClose: () => void;
}) {
  const { t } = useTranslation();
  return (
    <aside className="desktop-agent-panel">
      <Button
        aria-label={t('desktop.lobby.closeAgent')}
        icon={<X aria-hidden="true" />}
        onClick={onClose}
        size="compact"
        tone="quiet"
      >
        <span className="sr-only">{t('desktop.lobby.closeAgent')}</span>
      </Button>
      <ShieldCheck aria-hidden="true" />
      <p className="eyebrow">{t('desktop.lobby.verifiedAgent')}</p>
      <h2>{agent.displayName}</h2>
      <code>{agent.matrixUserId}</code>
      <dl>
        <div>
          <dt>{t('desktop.lobby.status')}</dt>
          <dd>{t(`lobby.status.${agent.status}`)}</dd>
        </div>
        <div>
          <dt>{t('desktop.lobby.instances')}</dt>
          <dd>{agent.instanceIds.length}</dd>
        </div>
      </dl>
    </aside>
  );
}
