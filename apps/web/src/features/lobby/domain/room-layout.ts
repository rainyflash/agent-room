import {
  characterSeed,
  isWalkableFloor,
  roomFloor,
  type FloorPoint,
  type RoomFloor,
} from './room-floor';

export type RoomPlacement = FloorPoint & { readonly slot: string };
export type RoomLayout = ReadonlyMap<string, RoomPlacement>;
export type PlacementRequest = {
  readonly id: string;
  readonly priority?: number;
  readonly preferred: {
    readonly x: number;
    readonly y: number;
    readonly width: number;
    readonly height: number;
  };
};

const stepX = 256;
const stepY = 240;
const firstX = 200;
const firstY = 280;

/** Expand the room instead of reducing the space available to each character. */
export function floorForOccupants(count: number, previous: RoomLayout = new Map()): RoomFloor {
  let width = roomFloor.width;
  let depth = roomFloor.depth;
  for (const point of previous.values()) {
    width = Math.max(width, Math.ceil((point.x + 180) / 256) * 256);
    depth = Math.max(depth, Math.ceil((point.y + 160) / 256) * 256);
  }
  while (slotCount(width, depth) < count) {
    if (width / depth < 1.5) width += 256;
    else depth += 256;
  }
  return Object.freeze({ width, depth });
}

/** Retain existing positions; deterministic free slots keep newcomers apart in bounded time. */
export function allocateRoomLayout(
  requests: readonly PlacementRequest[],
  previous: RoomLayout = new Map(),
  floor: RoomFloor = floorForOccupants(requests.length, previous),
): RoomLayout {
  const assigned = new Map<string, RoomPlacement>();
  const occupied = new Set<string>();
  for (const request of requests) {
    const saved = previous.get(request.id);
    if (saved !== undefined && isWalkableFloor(saved, 18, floor) && !occupied.has(saved.slot)) {
      assigned.set(request.id, saved);
      occupied.add(saved.slot);
    }
  }
  const slots = createSlots(floor).map((point) => ({ point, seed: characterSeed(point.slot) }));
  const additions = requests
    .filter((request) => !assigned.has(request.id))
    .map((request) => ({ request, seed: characterSeed(request.id) }))
    .toSorted(
      (a, b) =>
        (b.request.priority ?? 0) - (a.request.priority ?? 0) ||
        a.seed - b.seed ||
        a.request.id.localeCompare(b.request.id),
    );
  for (const { request, seed } of additions) {
    let best: RoomPlacement | undefined;
    let bestScore = Number.NEGATIVE_INFINITY;
    for (const candidate of slots) {
      const point = candidate.point;
      if (occupied.has(point.slot)) continue;
      const area = request.preferred;
      const preferred =
        point.x >= area.x &&
        point.x <= area.x + area.width &&
        point.y >= area.y &&
        point.y <= area.y + area.height;
      let clearance = 0;
      // Sparse rooms spread characters out; crowded rooms already have guaranteed grid spacing.
      if (requests.length <= 24) {
        clearance = 280;
        for (const other of assigned.values())
          clearance = Math.min(clearance, Math.hypot(other.x - point.x, other.y - point.y));
      }
      const score =
        clearance +
        (preferred ? 110 : 0) +
        // Human arrivals start by the entrance; agent positions have no business grouping.
        ((request.priority ?? 0) > 0
          ? -Math.hypot(point.x - area.x - area.width / 2, point.y - area.y - area.height / 2)
          : 0) +
        ((Math.imul(seed ^ candidate.seed, 1597334677) >>> 0) % 1000) / 25;
      if (score > bestScore) {
        best = point;
        bestScore = score;
      }
    }
    if (best === undefined)
      throw new Error('The room plan does not have enough walkable positions.');
    assigned.set(request.id, best);
    occupied.add(best.slot);
  }
  return assigned;
}

export function roamingRadius(): number {
  // Two neighbors can roam towards each other while preserving a clear clickable silhouette.
  return 18;
}

function slotCount(width: number, depth: number): number {
  return (
    (Math.floor((width - 280 - firstX) / stepX) + 1) *
    (Math.floor((depth - 220 - firstY) / stepY) + 1)
  );
}

function createSlots(floor: RoomFloor): readonly RoomPlacement[] {
  const result: RoomPlacement[] = [];
  for (let y = firstY; y <= floor.depth - 220; y += stepY) {
    for (let x = firstX; x <= floor.width - 280; x += stepX) {
      const slot = `${String(x)}:${String(y)}`;
      // Stable offsets soften rows while retaining space for bodies, labels and roaming.
      const seed = characterSeed(slot);
      result.push(
        Object.freeze({ x: x + (seed % 97) - 48, y: y + ((seed >>> 8) % 97) - 48, slot }),
      );
    }
  }
  return result;
}
