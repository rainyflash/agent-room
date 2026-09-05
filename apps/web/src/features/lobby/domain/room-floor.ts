export type FloorPoint = { readonly x: number; readonly y: number };
export type RoomFurnishing = FloorPoint & {
  readonly kind: 'desk' | 'table' | 'sofa' | 'plant' | 'shelf' | 'server';
  readonly width: number;
  readonly depth: number;
};

export const roomFloor = Object.freeze({ width: 1700, depth: 1000 });

export function projectFloorPoint(point: FloorPoint, elevation = 0): FloorPoint {
  return {
    x: 1100 + (point.x - point.y) * 0.82,
    y: 200 + (point.x + point.y) * 0.38 - elevation,
  };
}

export const roomFurnishings: readonly RoomFurnishing[] = Object.freeze([
  ...[210, 540, 870].flatMap((x) =>
    [180, 440].map((y) => ({ kind: 'desk' as const, x, y, width: 160, depth: 84 })),
  ),
  { kind: 'table', x: 1210, y: 360, width: 290, depth: 180 },
  { kind: 'sofa', x: 360, y: 790, width: 250, depth: 84 },
  { kind: 'table', x: 420, y: 680, width: 160, depth: 70 },
  { kind: 'shelf', x: 70, y: 45, width: 280, depth: 50 },
  { kind: 'server', x: 1210, y: 45, width: 100, depth: 70 },
  { kind: 'server', x: 1330, y: 45, width: 100, depth: 70 },
  ...[
    [70, 870],
    [1570, 80],
    [1530, 760],
    [1040, 690],
    [70, 370],
  ].map(([x = 0, y = 0]) => ({ kind: 'plant' as const, x, y, width: 60, depth: 60 })),
]);

export function isWalkableFloor(point: FloorPoint, margin = 18): boolean {
  return (
    point.x >= 60 &&
    point.x <= roomFloor.width - 60 &&
    point.y >= 80 &&
    point.y <= roomFloor.depth - 55 &&
    !roomFurnishings.some(
      (item) =>
        point.x > item.x - margin &&
        point.x < item.x + item.width + margin &&
        point.y > item.y - margin &&
        point.y < item.y + item.depth + margin,
    )
  );
}

export function nearbyWalkableFloor(point: FloorPoint): FloorPoint {
  if (isWalkableFloor(point)) return point;
  for (let distance = 24; distance <= 300; distance += 24) {
    for (let direction = 0; direction < 8; direction += 1) {
      const angle = (direction * Math.PI) / 4;
      const candidate = {
        x: point.x + Math.cos(angle) * distance,
        y: point.y + Math.sin(angle) * distance,
      };
      if (isWalkableFloor(candidate)) return candidate;
    }
  }
  return { x: 1080, y: 900 };
}

export function characterSeed(value: string): number {
  let seed = 2166136261;
  for (const character of value) seed = Math.imul(seed ^ (character.codePointAt(0) ?? 0), 16777619);
  return seed >>> 0;
}
