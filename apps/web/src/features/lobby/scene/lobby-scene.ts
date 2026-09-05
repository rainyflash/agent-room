import type { SceneFrame } from './scene-character';
import type { LobbySceneProjection, LobbyZoneId } from '@/features/lobby/domain/scene-projection';

export type LobbySceneLabels = {
  readonly canvas: string;
  readonly self?: string;
  readonly zones: Readonly<Record<LobbyZoneId, string>>;
};

export type LobbySceneCallbacks = {
  readonly onFrame?: (frame: SceneFrame) => void;
  readonly onSelectHuman?: (matrixUserId: string) => void;
  readonly onSelectAgent: (agentId: string | null) => void;
  readonly onZoomChange: (zoom: number) => void;
};

export type LobbySceneMountOptions = LobbySceneCallbacks & {
  readonly host: HTMLElement;
  readonly labels: LobbySceneLabels;
  readonly projection: LobbySceneProjection;
};

export type LobbySceneHandle = {
  destroy(): void;
  resetViewport(): void;
  focusAgent?(agentId: string): void;
  update(projection: LobbySceneProjection): void;
  zoomBy(factor: number): void;
};
