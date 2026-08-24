import type {
  LobbyAgentNodeProjection,
  LobbySceneProjection,
  LobbyZoneId,
} from '@/features/lobby/domain/scene-projection';

export type LobbySceneLabels = {
  readonly agentAccessibilityLabel: (agent: LobbyAgentNodeProjection) => string;
  readonly canvas: string;
  readonly zones: Readonly<Record<LobbyZoneId, string>>;
};

export type LobbySceneCallbacks = {
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
  update(projection: LobbySceneProjection): void;
  zoomBy(factor: number): void;
};
