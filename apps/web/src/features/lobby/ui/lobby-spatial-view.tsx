import type { RoomSpeech } from '../domain/room-speech';
import type { LobbySceneProjection } from '../domain/scene-projection';
import { useImperativeHandle, useMemo, useRef, useState, type Ref } from 'react';
import { useTranslation } from 'react-i18next';
import { useAppServices } from '@/app/app-services';
import type { LobbyRoom } from '@/features/lobby/domain/lobby';
import type { LobbySceneLabels } from '@/features/lobby/scene/lobby-scene';
import { ListModeRoster, type ListModeRosterHandle } from '@/features/lobby/ui/list-mode-roster';
import {
  LobbySceneSurface,
  type LobbySceneSurfaceHandle,
} from '@/features/lobby/ui/lobby-scene-surface';
import { SignalDock } from '@/features/lobby/ui/signal-dock';
import { useListModeRequirement } from '@/features/lobby/ui/use-list-mode-requirement';
import { useAccountPreferences } from '@/features/preferences/ui/account-preferences-provider';
import { resolveFrontendSurface } from '@/features/telemetry/adapters/runtime-surface';

export type LobbySpatialViewHandle = { focus(): void };

export function LobbySpatialView({
  room,
  projection,
  speech,
  onOpenSpeech,
  onSelectHuman,
  selectedAgentId,
  onSelectAgent,
  ref,
}: {
  readonly room: LobbyRoom;
  readonly projection: LobbySceneProjection;
  readonly speech: readonly RoomSpeech[];
  readonly onOpenSpeech: (id: string) => void;
  readonly onSelectHuman: (id: string) => void;
  readonly selectedAgentId: string | null;
  readonly onSelectAgent: (id: string | null) => void;
  readonly ref: Ref<LobbySpatialViewHandle>;
}) {
  const { i18n, t } = useTranslation();
  const { telemetry } = useAppServices();
  const preferences = useAccountPreferences();
  const requirement = useListModeRequirement();
  const scene = useRef<LobbySceneSurfaceHandle>(null);
  const list = useRef<ListModeRosterHandle>(null);
  const [zoom, setZoom] = useState(1);
  const mode = requirement === null ? preferences.snapshot.values.lobbyView : 'list';
  const languageKey = i18n.resolvedLanguage ?? i18n.language;
  const labels = useMemo<LobbySceneLabels>(
    () => ({
      canvas: t('lobby.scene.canvasLabel'),
      self: t('roomGame.self'),
      zones: {
        active: t('lobby.zone.active'),
        attention: t('lobby.zone.attention'),
        available: t('lobby.zone.available'),
      },
    }),
    [languageKey, t],
  );
  useImperativeHandle(ref, () => ({
    focus: () => (mode === 'scene' ? scene.current?.focus() : list.current?.focusSelected()),
  }));
  return (
    <div className="workspace-space">
      <div className={`lobby-stage lobby-stage--${mode}`}>
        {mode === 'scene' ? (
          <LobbySceneSurface
            labels={labels}
            speech={speech}
            onOpenSpeech={onOpenSpeech}
            onSelectHuman={onSelectHuman}
            languageKey={languageKey}
            onSceneInitialized={(durationMs) => {
              void telemetry.record({
                metric: 'scene_initialization',
                surface: resolveFrontendSurface(),
                value: durationMs,
              });
            }}
            onSelectAgent={onSelectAgent}
            onZoomChange={setZoom}
            projection={projection}
            ref={scene}
          />
        ) : (
          <>
            <p className="workspace-space__notice">
              {requirement === null || requirement === 'compact'
                ? t('lobby.roster.eyebrow')
                : t(`lobby.scene.listMode.${requirement}`)}
            </p>
            <ListModeRoster
              agents={room.agents}
              onSelectAgent={onSelectAgent}
              selectedAgentId={selectedAgentId}
              ref={list}
            />
          </>
        )}
      </div>
      <SignalDock
        mode={mode}
        onModeChange={(next) => {
          if (next !== 'scene' || requirement === null) preferences.setLobbyView(next);
        }}
        onResetViewport={() => scene.current?.resetViewport()}
        onZoomBy={(factor) => scene.current?.zoomBy(factor)}
        sceneAvailable={requirement === null}
        zoom={zoom}
      />
    </div>
  );
}
