import { allocateRoomLayout, roamingRadius, type RoomLayout } from './room-layout';
import type { RoomHuman } from './room-participants';
import { projectFloorPoint, type FloorPoint } from './room-floor';
import type { LobbyAgent, LobbyAgentStatus, LobbyRoom } from './lobby';

export const lobbyZoneIds = ['active', 'attention', 'available'] as const;

export type LobbyZoneId = (typeof lobbyZoneIds)[number];
export type LobbySceneDetail = 'distant' | 'medium' | 'near';

export type LobbyWorld = {
  readonly height: number;
  readonly width: number;
};

export type LobbyBounds = {
  readonly height: number;
  readonly width: number;
  readonly x: number;
  readonly y: number;
};

export type LobbyZoneProjection = LobbyBounds & {
  readonly id: LobbyZoneId;
};

export type LobbyAgentNodeProjection = LobbyAgent & {
  readonly radius: number;
  readonly roamingRadius?: number;
  readonly floorPosition?: FloorPoint;
  readonly x: number;
  readonly y: number;
  readonly zoneId: LobbyZoneId;
};

export type LobbyHumanNodeProjection = RoomHuman & {
  readonly characterId: string;
  readonly floorPosition: FloorPoint;
  readonly radius: number;
  readonly x: number;
  readonly y: number;
};

export type LobbySceneProjection = {
  readonly humans?: readonly LobbyHumanNodeProjection[];
  readonly layout?: RoomLayout;
  readonly nodes: readonly LobbyAgentNodeProjection[];
  readonly observedAtUnixMs: number;
  readonly roomId: string;
  readonly roomName: string;
  readonly selectedAgentId: string | null;
  readonly topic?: string;
  readonly world: LobbyWorld;
  readonly zones: readonly LobbyZoneProjection[];
};

export type LobbyViewport = LobbyBounds & {
  readonly zoom: number;
};

const WORLD: LobbyWorld = Object.freeze({ height: 1_500, width: 2_600 });

const ZONES: Readonly<Record<LobbyZoneId, LobbyZoneProjection>> = Object.freeze({
  active: Object.freeze({ height: 520, id: 'active', width: 970, x: 100, y: 110 }),
  attention: Object.freeze({ height: 640, id: 'attention', width: 460, x: 1130, y: 180 }),
  available: Object.freeze({ height: 270, id: 'available', width: 970, x: 130, y: 670 }),
});

const ZONE_BY_STATUS: Readonly<Record<LobbyAgentStatus, LobbyZoneId>> = Object.freeze({
  blocked: 'attention',
  completed: 'active',
  idle: 'available',
  offline: 'available',
  waiting_input: 'attention',
  working: 'active',
});

export function projectLobbyScene(
  room: LobbyRoom,
  selectedAgentId: string | null,
  options: { readonly previous?: RoomLayout; readonly humans?: readonly RoomHuman[] } = {},
): LobbySceneProjection & { readonly layout: RoomLayout } {
  const humans = options.humans ?? [];
  const requests = [
    ...room.agents.map((agent) => ({
      id: agent.agentId,
      preferred: ZONES[ZONE_BY_STATUS[agent.status]],
    })),
    ...humans.map((human) => ({
      id: `human:${human.matrixUserId}`,
      preferred: { x: 1000, y: 830, width: 600, height: 100 },
    })),
  ];
  const layout = allocateRoomLayout(requests, options.previous);
  const nodes = room.agents
    .toSorted((a, b) => a.agentId.localeCompare(b.agentId))
    .flatMap((agent): LobbyAgentNodeProjection[] => {
      const floor = layout.get(agent.agentId);
      if (floor === undefined) return [];
      return [
        Object.freeze({
          ...agent,
          radius: 26,
          floorPosition: floor,
          roamingRadius: roamingRadius(floor, layout),
          ...projectFloorPoint(floor),
          zoneId: ZONE_BY_STATUS[agent.status],
        }),
      ];
    });
  const humanNodes = humans.flatMap((human): LobbyHumanNodeProjection[] => {
    const characterId = `human:${human.matrixUserId}`;
    const floor = layout.get(characterId);
    return floor === undefined
      ? []
      : [
          Object.freeze({
            ...human,
            characterId,
            radius: 28,
            floorPosition: floor,
            ...projectFloorPoint(floor),
          }),
        ];
  });
  return Object.freeze({
    nodes: Object.freeze(nodes),
    humans: Object.freeze(humanNodes),
    layout,
    observedAtUnixMs: room.observedAtUnixMs,
    roomId: room.roomId,
    roomName: room.name,
    selectedAgentId: nodes.some((node) => node.agentId === selectedAgentId)
      ? selectedAgentId
      : null,
    ...(room.topic === undefined ? {} : { topic: room.topic }),
    world: WORLD,
    zones: Object.freeze(lobbyZoneIds.map((zoneId) => ZONES[zoneId])),
  });
}

export function visibleLobbyNodes(
  projection: LobbySceneProjection,
  viewport: LobbyViewport,
): readonly LobbyAgentNodeProjection[] {
  const right = viewport.x + viewport.width;
  const bottom = viewport.y + viewport.height;
  return projection.nodes.filter((node) => {
    return (
      node.x + node.radius >= viewport.x &&
      node.x - node.radius <= right &&
      node.y + node.radius >= viewport.y &&
      node.y - node.radius <= bottom
    );
  });
}

export function sceneDetailForZoom(zoom: number): LobbySceneDetail {
  if (!Number.isFinite(zoom) || zoom < 0.68) {
    return 'distant';
  }
  return zoom < 1.18 ? 'medium' : 'near';
}
