import type { SceneFrame, SceneCharacter } from './scene-character';
import type { LobbyAgentStatus } from '../domain/lobby';
import type { LobbySceneProjection, LobbyZoneId } from '@/features/lobby/domain/scene-projection';

export type LobbySceneLabels = {
  readonly canvas: string;
  readonly self?: string;
  readonly statuses?: Readonly<Record<LobbyAgentStatus | 'present', string>>;
  readonly availability?: Readonly<
    Partial<Record<NonNullable<SceneCharacter['availability']>, string>>
  >;
  readonly zones: Readonly<Record<LobbyZoneId, string>>;
};

/**
 * 名牌只放一行大白话：任务状态在前，接待能力确定时再补一句。接待未知时不写，
 * 免得把「还不知道」说成一种状态；离线和重连本身就是状态，直接替换。
 */
export function characterStatusLabel(
  node: SceneCharacter,
  labels: LobbySceneLabels,
): string | undefined {
  const status = labels.statuses?.[node.status];
  const availability = node.availability;
  if (availability === undefined || availability === 'unknown') return status;
  const reception = labels.availability?.[availability];
  if (reception === undefined) return status;
  if (availability === 'offline' || availability === 'reconnecting' || status === undefined)
    return reception;
  return `${status} · ${reception}`;
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
