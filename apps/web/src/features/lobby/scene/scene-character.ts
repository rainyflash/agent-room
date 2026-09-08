import type { LobbyAgentStatus } from '../domain/lobby';
import type { FloorPoint, RoomFloor } from '../domain/room-floor';
import type { LobbySceneProjection, LobbyViewport } from '../domain/scene-projection';
import { agentReception, type AgentReception } from '../domain/agent-attendance';

export type SceneCharacter = {
  readonly characterId: string;
  readonly matrixUserId: string;
  readonly displayName: string;
  readonly kind: 'agent' | 'human';
  readonly isSelf: boolean;
  readonly status: LobbyAgentStatus | 'present';
  readonly reception?: AgentReception;
  readonly radius: number;
  readonly roamingRadius?: number;
  readonly floorPosition?: FloorPoint;
  readonly floor?: RoomFloor;
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
      reception: agentReception(node, scene.observedAtUnixMs),
      floor: { width: scene.world.width, depth: scene.world.height },
    })),
    ...(scene.humans ?? []).map((human): SceneCharacter => ({
      ...human,
      displayName:
        human.isSelf && selfLabel.length > 0
          ? `${selfLabel} · ${human.displayName}`
          : human.displayName,
      kind: 'human',
      status: 'present',
      roamingRadius: 0,
    })),
  ];
}

export type SceneFrame = {
  readonly viewport?: LobbyViewport;
  readonly width: number;
  readonly height: number;
  readonly characters: readonly {
    readonly characterId: string;
    readonly x: number;
    readonly y: number;
  }[];
};
