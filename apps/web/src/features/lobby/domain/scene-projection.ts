import {
  allocateRoomLayout,
  floorForOccupants,
  roamingRadius,
  type RoomLayout,
} from './room-layout';
import type { RoomHuman } from './room-participants';
import { projectFloorPoint, type FloorPoint } from './room-floor';
import type { LobbyAgent, LobbyAgentStatus, LobbyRoom } from './lobby';
import { agentAttendance } from './agent-attendance';

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

function zonesForWorld(world: LobbyWorld): Readonly<Record<LobbyZoneId, LobbyZoneProjection>> {
  const width = world.width - 380;
  const height = world.height - 420;
  return {
    active: { id: 'active', x: 180, y: 240, width: width * 0.48, height: height * 0.48 },
    attention: {
      id: 'attention',
      x: 180 + width * 0.52,
      y: 240,
      width: width * 0.48,
      height: height * 0.48,
    },
    available: { id: 'available', x: 180, y: 260 + height * 0.52, width, height: height * 0.48 },
  };
}

const ZONE_BY_STATUS: Readonly<Record<LobbyAgentStatus, LobbyZoneId>> = Object.freeze({
  blocked: 'attention',
  completed: 'available',
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
  const presentAgents = room.agents.filter(
    (agent) => agentAttendance(agent, room.observedAtUnixMs) !== 'away',
  );
  const floorPlan = floorForOccupants(presentAgents.length + humans.length, options.previous);
  const world = Object.freeze({ width: floorPlan.width, height: floorPlan.depth });
  const zones = zonesForWorld(world);
  const requests = [
    ...presentAgents.map((agent) => ({
      id: agent.agentId,
      preferred: { x: 180, y: 240, width: world.width - 460, height: world.height - 460 },
    })),
    ...humans.map((human) => ({
      id: `human:${human.matrixUserId}`,
      priority: human.isSelf ? 2 : 1,
      preferred: { x: world.width / 2 - 300, y: world.height - 410, width: 600, height: 200 },
    })),
  ];
  const layout = allocateRoomLayout(requests, options.previous, floorPlan);
  const nodes = presentAgents
    .toSorted((a, b) => a.agentId.localeCompare(b.agentId))
    .flatMap((agent): LobbyAgentNodeProjection[] => {
      const floor = layout.get(agent.agentId);
      if (floor === undefined) return [];
      return [
        Object.freeze({
          ...agent,
          reportedStatus: agent.reportedStatus ?? agent.status,
          status:
            agentAttendance(agent, room.observedAtUnixMs) === 'reconnecting'
              ? 'offline'
              : agent.status,
          radius: presentAgents.length <= 24 ? 34 : 26,
          floorPosition: floor,
          roamingRadius: roamingRadius(),
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
            radius: presentAgents.length <= 24 ? 34 : 28,
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
    world,
    zones: Object.freeze(lobbyZoneIds.map((zoneId) => Object.freeze(zones[zoneId]))),
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
  if (!Number.isFinite(zoom) || zoom < 0.4) {
    return 'distant';
  }
  return zoom < 0.82 ? 'medium' : 'near';
}
