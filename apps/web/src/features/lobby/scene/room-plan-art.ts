import { furnishingsForFloor } from '../domain/room-floor';
import type { LobbyWorld } from '../domain/scene-projection';

export type RoomPlanShape =
  | {
      readonly kind: 'rect';
      readonly x: number;
      readonly y: number;
      readonly width: number;
      readonly height: number;
      readonly radius: number;
      readonly fill: string;
      readonly stroke: string;
    }
  | {
      readonly kind: 'ellipse';
      readonly x: number;
      readonly y: number;
      readonly rx: number;
      readonly ry: number;
      readonly fill: string;
      readonly stroke: string;
    }
  | {
      readonly kind: 'line';
      readonly x: number;
      readonly y: number;
      readonly toX: number;
      readonly toY: number;
      readonly stroke: string;
    };

/** Shared, scalable floor-plan geometry for both renderers; characters remain separate sprites. */
export function roomPlanShapes(world: LobbyWorld): readonly RoomPlanShape[] {
  const shapes: RoomPlanShape[] = [];
  const rect = (
    x: number,
    y: number,
    width: number,
    height: number,
    radius = 8,
    fill = '#fafbf9',
    stroke = '#c9d0d1',
  ): void => {
    shapes.push({ kind: 'rect', x, y, width, height, radius, fill, stroke });
  };
  const ellipse = (
    x: number,
    y: number,
    rx: number,
    ry: number,
    fill = '#fafbf9',
    stroke = '#c9d0d1',
  ): void => {
    shapes.push({ kind: 'ellipse', x, y, rx, ry, fill, stroke });
  };
  const line = (x: number, y: number, toX: number, toY: number): void => {
    shapes.push({ kind: 'line', x, y, toX, toY, stroke: '#aab5b7' });
  };
  rect(22, 22, world.width - 44, world.height - 44, 0, '#fff', 'transparent');
  line(22, 22, world.width - 22, 22);
  line(22, 22, 22, world.height - 22);
  line(world.width - 22, 22, world.width - 22, world.height - 22);
  line(22, world.height - 22, world.width / 2 - 120, world.height - 22);
  line(world.width / 2 + 120, world.height - 22, world.width - 22, world.height - 22);
  for (const item of furnishingsForFloor({ width: world.width, depth: world.height })) {
    if (item.kind === 'desk') {
      rect(item.x + 62, item.y + 50, 40, 32, 10);
      rect(item.x, item.y, item.width, 54, 16);
      rect(item.x + 58, item.y + 12, 48, 26, 3, '#eff3f2');
      ellipse(item.x + 25, item.y + 30, 7, 7, '#d5e6df', '#799b91');
    } else if (item.kind === 'table') {
      rect(item.x + 24, item.y, 62, 35, 14);
      rect(item.x + 24, item.y + 112, 62, 35, 14);
      ellipse(item.x + 56, item.y + 74, 54, 54);
      ellipse(item.x + 56, item.y + 74, 9, 9, '#d5e6df', '#799b91');
    } else if (item.kind === 'sofa') {
      rect(item.x, item.y, item.width, item.depth, 16);
      rect(item.x + 4, item.y + 8, 10, item.depth - 16, 4, '#f3f6f4');
    } else {
      ellipse(item.x + 20, item.y + 24, 18, 16);
      ellipse(item.x + 14, item.y + 13, 7, 13, '#a9c8ba', '#799b91');
      ellipse(item.x + 27, item.y + 12, 6, 11, '#a9c8ba', '#799b91');
    }
  }
  return shapes;
}
