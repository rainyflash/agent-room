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
  readonly x: number;
  readonly y: number;
  readonly zoneId: LobbyZoneId;
};

export type LobbySceneProjection = {
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
  active: Object.freeze({ height: 800, id: 'active', width: 1_500, x: 80, y: 120 }),
  attention: Object.freeze({ height: 800, id: 'attention', width: 880, x: 1_640, y: 120 }),
  available: Object.freeze({ height: 400, id: 'available', width: 1_720, x: 400, y: 980 }),
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
): LobbySceneProjection {
  const nodes = lobbyZoneIds.flatMap((zoneId) => {
    const agents = room.agents
      .filter((agent) => ZONE_BY_STATUS[agent.status] === zoneId)
      .toSorted(compareAgents);
    return layoutZone(agents, ZONES[zoneId]);
  });
  const selected = nodes.some((node) => node.agentId === selectedAgentId) ? selectedAgentId : null;
  return Object.freeze({
    nodes: Object.freeze(nodes),
    observedAtUnixMs: room.observedAtUnixMs,
    roomId: room.roomId,
    roomName: room.name,
    selectedAgentId: selected,
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

function layoutZone(
  agents: readonly LobbyAgent[],
  zone: LobbyZoneProjection,
): LobbyAgentNodeProjection[] {
  if (agents.length === 0) {
    return [];
  }
  const contentBounds = {
    height: Math.max(1, zone.height - 82),
    width: Math.max(1, zone.width - 36),
    x: zone.x + 18,
    y: zone.y + 62,
  };
  const aspectRatio = contentBounds.width / contentBounds.height;
  const columns = Math.max(1, Math.ceil(Math.sqrt(agents.length * aspectRatio)));
  const rows = Math.ceil(agents.length / columns);
  const cellWidth = contentBounds.width / columns;
  const cellHeight = contentBounds.height / rows;
  const radius = clamp(Math.min(cellWidth, cellHeight) * 0.24, 18, 34);
  const jitterLimit = Math.min(cellWidth, cellHeight) * 0.08;

  return agents.map((agent, index) => {
    const column = index % columns;
    const row = Math.floor(index / columns);
    const seed = stableHash(agent.agentId);
    const jitterX = signedUnit(seed) * jitterLimit;
    const jitterY = signedUnit(mixHash(seed)) * jitterLimit;
    return Object.freeze({
      ...agent,
      radius,
      x: contentBounds.x + (column + 0.5) * cellWidth + jitterX,
      y: contentBounds.y + (row + 0.5) * cellHeight + jitterY,
      zoneId: zone.id,
    });
  });
}

function compareAgents(left: LobbyAgent, right: LobbyAgent): number {
  const hashDifference = stableHash(left.agentId) - stableHash(right.agentId);
  return hashDifference === 0 ? left.agentId.localeCompare(right.agentId) : hashDifference;
}

function stableHash(value: string): number {
  let hash = 2_166_136_261;
  for (const character of value) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16_777_619);
  }
  return hash >>> 0;
}

function mixHash(value: number): number {
  let mixed = value ^ (value >>> 16);
  mixed = Math.imul(mixed, 0x7feb_352d);
  mixed ^= mixed >>> 15;
  mixed = Math.imul(mixed, 0x846c_a68b);
  return (mixed ^ (mixed >>> 16)) >>> 0;
}

function signedUnit(value: number): number {
  return (value / 0xffff_ffff) * 2 - 1;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), maximum);
}
