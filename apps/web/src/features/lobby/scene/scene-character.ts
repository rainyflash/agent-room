import type { FloorPoint, RoomFloor } from '../domain/room-floor';
import type { LobbySceneProjection, LobbyViewport } from '../domain/scene-projection';
import {
  agentReception,
  agentStateKey,
  type AgentReception,
  type AgentStateKey,
} from '../domain/agent-attendance';

export type SceneCharacter = {
  readonly characterId: string;
  readonly matrixUserId: string;
  readonly displayName: string;
  readonly kind: 'agent' | 'human';
  readonly isSelf: boolean;
  readonly reception?: AgentReception;
  /** 名牌下的贴纸和头顶圆点看它：在不在等消息，或者重连中、离线。人没有。 */
  readonly availability?: AgentStateKey;
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
      availability: agentStateKey(node, scene.observedAtUnixMs),
      floor: { width: scene.world.width, depth: scene.world.height },
    })),
    ...(scene.humans ?? []).map((human): SceneCharacter => ({
      ...human,
      displayName:
        human.isSelf && selfLabel.length > 0
          ? `${selfLabel} · ${human.displayName}`
          : human.displayName,
      kind: 'human',
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
