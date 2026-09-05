import { characterSeed, isWalkableFloor, projectFloorPoint, type FloorPoint } from './room-floor';

export type RoomPlacement = FloorPoint & { readonly slot: string };
export type RoomLayout = ReadonlyMap<string, RoomPlacement>;
export type PlacementRequest = {
  readonly id: string;
  readonly preferred: {
    readonly x: number;
    readonly y: number;
    readonly width: number;
    readonly height: number;
  };
};

const slots = createSlots(48, 56);
const overflowSlots = createSlots(24, 28).filter(
  (slot) => !slots.some((coarse) => coarse.slot === slot.slot),
);

/** 保留房间中已有角色的位置，只为新加入的角色选择空位。 */
export function allocateRoomLayout(
  requests: readonly PlacementRequest[],
  previous: RoomLayout = new Map(),
): RoomLayout {
  const assigned = new Map<string, RoomPlacement>();
  const occupied = new Set<string>();
  for (const request of requests) {
    const saved = previous.get(request.id);
    if (saved !== undefined && isWalkableFloor(saved) && !occupied.has(saved.slot)) {
      assigned.set(request.id, saved);
      occupied.add(saved.slot);
    }
  }
  const additions = requests
    .filter((request) => !assigned.has(request.id))
    .toSorted((a, b) => characterSeed(a.id) - characterSeed(b.id) || a.id.localeCompare(b.id));
  for (const request of additions) {
    const available = slots.some((slot) => !occupied.has(slot.slot)) ? slots : overflowSlots;
    let slot: RoomPlacement | null = null;
    let bestScore = Number.NEGATIVE_INFINITY;
    for (const candidate of available) {
      if (occupied.has(candidate.slot)) continue;
      const score = placementScore(candidate, request, assigned);
      if (score > bestScore) {
        slot = candidate;
        bestScore = score;
      }
    }
    if (slot === null) continue;
    assigned.set(request.id, slot);
    occupied.add(slot.slot);
  }
  return assigned;
}

export function roamingRadius(point: FloorPoint, layout: RoomLayout): number {
  const screen = projectFloorPoint(point);
  let distance = 130;
  for (const other of layout.values()) {
    if (other.x === point.x && other.y === point.y) continue;
    const projected = projectFloorPoint(other);
    distance = Math.min(distance, Math.hypot(projected.x - screen.x, projected.y - screen.y));
  }
  return Math.max(0, Math.min(30, (distance - 48) / 2));
}

function placementScore(
  point: RoomPlacement,
  request: PlacementRequest,
  assigned: RoomLayout,
): number {
  const screen = projectFloorPoint(point);
  let clearance = 160;
  for (const other of assigned.values()) {
    const projected = projectFloorPoint(other);
    clearance = Math.min(clearance, Math.hypot(projected.x - screen.x, projected.y - screen.y));
  }
  const area = request.preferred;
  const preferred =
    point.x >= area.x &&
    point.x <= area.x + area.width &&
    point.y >= area.y &&
    point.y <= area.y + area.height;
  return (
    clearance + (preferred ? 65 : 0) + (characterSeed(`${request.id}:${point.slot}`) % 1000) / 40
  );
}

function createSlots(stepX: number, stepY: number): readonly RoomPlacement[] {
  const result: RoomPlacement[] = [];
  for (let screenY = 250; screenY <= 1200; screenY += stepY) {
    for (let screenX = 300; screenX <= 2450; screenX += stepX) {
      const point = {
        x: ((screenX - 1100) / 0.82 + (screenY - 200) / 0.38) / 2,
        y: ((screenY - 200) / 0.38 - (screenX - 1100) / 0.82) / 2,
        slot: `${String(screenX)}:${String(screenY)}`,
      };
      if (isWalkableFloor(point, 22)) result.push(Object.freeze(point));
    }
  }
  return Object.freeze(result);
}
