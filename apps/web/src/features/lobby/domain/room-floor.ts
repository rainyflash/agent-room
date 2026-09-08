export type FloorPoint = { readonly x: number; readonly y: number };
export type RoomFloor = { readonly width: number; readonly depth: number };
export type RoomFurnishing = FloorPoint & {
  readonly kind: 'desk' | 'table' | 'sofa' | 'plant';
  readonly width: number;
  readonly depth: number;
};

export const roomFloor: RoomFloor = Object.freeze({ width: 1536, depth: 1024 });

export function projectFloorPoint(point: FloorPoint, elevation = 0): FloorPoint {
  return { x: point.x, y: point.y - elevation };
}

export function unprojectFloorPoint(point: FloorPoint): FloorPoint {
  return { ...point };
}

export function furnishingsForFloor(floor: RoomFloor): readonly RoomFurnishing[] {
  return [
    ...[0.23, 0.5, 0.77].map((portion): RoomFurnishing => ({
      kind: 'desk',
      x: floor.width * portion - 82,
      y: 64,
      width: 164,
      depth: 90,
    })),
    { kind: 'table', x: floor.width - 156, y: floor.depth * 0.5 - 72, width: 112, depth: 144 },
    { kind: 'sofa', x: 44, y: floor.depth * 0.5 - 72, width: 58, depth: 144 },
    { kind: 'plant', x: floor.width - 100, y: 70, width: 42, depth: 42 },
    { kind: 'sofa', x: floor.width / 2 - 450, y: floor.depth - 100, width: 180, depth: 58 },
    { kind: 'sofa', x: floor.width / 2 + 270, y: floor.depth - 100, width: 180, depth: 58 },
    { kind: 'plant', x: floor.width / 2 - 510, y: floor.depth - 95, width: 42, depth: 42 },
    { kind: 'plant', x: floor.width / 2 + 470, y: floor.depth - 95, width: 42, depth: 42 },
  ];
}

export const roomFurnishings = furnishingsForFloor(roomFloor);

export function isWalkableFloor(point: FloorPoint, margin = 18, floor = roomFloor): boolean {
  return (
    point.x >= 140 &&
    point.x <= floor.width - 180 &&
    point.y >= 220 &&
    point.y <= floor.depth - 100 &&
    !furnishingsForFloor(floor).some(
      (item) =>
        point.x > item.x - margin &&
        point.x < item.x + item.width + margin &&
        point.y > item.y - margin &&
        point.y < item.y + item.depth + margin,
    )
  );
}

export function nearbyWalkableFloor(point: FloorPoint): FloorPoint {
  return {
    x: Math.max(160, Math.min(roomFloor.width - 200, point.x)),
    y: Math.max(240, Math.min(roomFloor.depth - 120, point.y)),
  };
}

export function characterSeed(value: string): number {
  let seed = 2166136261;
  for (const character of value) seed = Math.imul(seed ^ (character.codePointAt(0) ?? 0), 16777619);
  return seed >>> 0;
}
