import type { SceneFrame, SceneCharacter } from './scene-character';
import type { AgentStateKey } from '../domain/agent-attendance';
import type { LobbySceneProjection } from '@/features/lobby/domain/scene-projection';

export type LobbySceneLabels = {
  readonly canvas: string;
  readonly self?: string;
  readonly availability?: Readonly<Partial<Record<AgentStateKey, string>>>;
};

/**
 * 名牌下只放一行大白话：在不在等消息，重连中、离线就直接说。接待未知时不写，
 * 免得把「还不知道」说成一种状态。
 */
export function characterStatusLabel(
  node: SceneCharacter,
  labels: LobbySceneLabels,
): string | undefined {
  const availability = node.availability;
  if (availability === undefined || availability === 'unknown') return undefined;
  return labels.availability?.[availability];
}

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
  releaseFocus(): void;
  focusAgent?(agentId: string): void;
  focusArea?(x: number, y: number): void;
  update(projection: LobbySceneProjection): void;
  zoomBy(factor: number): void;
};
