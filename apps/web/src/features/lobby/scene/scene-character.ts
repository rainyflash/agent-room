import type { LobbyAgentStatus } from '../domain/lobby';
import type { FloorPoint } from '../domain/room-floor';
import type { LobbySceneProjection } from '../domain/scene-projection';

export type SceneCharacter = {
  readonly characterId: string;
  readonly matrixUserId: string;
  readonly displayName: string;
  readonly kind: 'agent' | 'human';
  readonly isSelf: boolean;
  readonly status: LobbyAgentStatus | 'present';
  readonly radius: number;
  readonly roamingRadius?: number;
  readonly floorPosition?: FloorPoint;
  readonly x: number;
  readonly y: number;
};

export function sceneCharacters(
  scene: LobbySceneProjection,
  selfLabel = '',
): readonly SceneCharacter[] {
  return [
    ...scene.nodes.map((node): SceneCharacter => ({
      ...node,
      characterId: node.agentId,
      kind: 'agent',
      isSelf: false,
    })),
    ...(scene.humans ?? []).map((human): SceneCharacter => ({
      ...human,
      displayName:
        human.isSelf && selfLabel.length > 0
          ? `${human.displayName} · ${selfLabel}`
          : human.displayName,
      kind: 'human',
      status: 'present',
      roamingRadius: 0,
    })),
  ];
}

export type SceneFrame = {
  readonly width: number;
  readonly height: number;
  readonly characters: readonly {
    readonly characterId: string;
    readonly x: number;
    readonly y: number;
  }[];
};
