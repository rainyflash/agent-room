import { furnishingsForFloor } from '../domain/room-floor';
import type { LobbyWorld } from '../domain/scene-projection';
import { sceneInk, sceneStrokeWidth } from './scene-style';

export type RoomPlanShape =
  | {
      /** 地砖棋盘格：由各渲染器用平铺纹理或 SVG 图案绘制，房间再大也只占一个图元。 */
      readonly kind: 'floor';
      readonly x: number;
      readonly y: number;
      readonly width: number;
      readonly height: number;
    }
  | {
      readonly kind: 'rect';
      readonly x: number;
      readonly y: number;
      readonly width: number;
      readonly height: number;
      readonly radius: number;
      readonly fill: string;
      readonly stroke: string;
      readonly strokeWidth: number;
    }
  | {
      readonly kind: 'ellipse';
      readonly x: number;
      readonly y: number;
      readonly rx: number;
      readonly ry: number;
      readonly fill: string;
      readonly stroke: string;
      readonly strokeWidth: number;
    }
  | {
      readonly kind: 'line';
      readonly x: number;
      readonly y: number;
      readonly toX: number;
      readonly toY: number;
      readonly stroke: string;
      readonly strokeWidth: number;
    };

const furniture = {
  wood: '#e9b27a',
  seat: '#ffc53d',
  chair: '#8ec5f5',
  screen: '#e7e9ee',
  sofa: '#7fc4a8',
  cushion: '#a5dcc5',
  leaf: '#6cc08b',
  pot: '#f28b6e',
  wall: '#f28b6e',
} as const;

/** Shared, scalable floor-plan geometry for both renderers; characters remain separate sprites. */
export function roomPlanShapes(world: LobbyWorld): readonly RoomPlanShape[] {
  const shapes: RoomPlanShape[] = [];
  const rect = (
    x: number,
    y: number,
    width: number,
    height: number,
    radius: number,
    fill: string,
  ): void => {
    shapes.push({
      kind: 'rect',
      x,
      y,
      width,
      height,
      radius,
      fill,
      stroke: sceneInk,
      strokeWidth: sceneStrokeWidth.furniture,
    });
  };
  const ellipse = (x: number, y: number, rx: number, ry: number, fill: string): void => {
    shapes.push({
      kind: 'ellipse',
      x,
      y,
      rx,
      ry,
      fill,
      stroke: sceneInk,
      strokeWidth: sceneStrokeWidth.furniture,
    });
  };
  const wall = (x: number, y: number, toX: number, toY: number): void => {
    shapes.push({
      kind: 'line',
      x,
      y,
      toX,
      toY,
      stroke: sceneInk,
      strokeWidth: sceneStrokeWidth.wall,
    });
  };
  shapes.push({ kind: 'floor', x: 22, y: 22, width: world.width - 44, height: world.height - 44 });
  rect(22, 22, world.width - 44, 26, 0, furniture.wall);
  wall(22, 22, world.width - 22, 22);
  wall(22, 22, 22, world.height - 22);
  wall(world.width - 22, 22, world.width - 22, world.height - 22);
  wall(22, world.height - 22, world.width / 2 - 120, world.height - 22);
  wall(world.width / 2 + 120, world.height - 22, world.width - 22, world.height - 22);
  for (const item of furnishingsForFloor({ width: world.width, depth: world.height })) {
    if (item.kind === 'desk') {
      rect(item.x + 62, item.y + 50, 40, 32, 10, furniture.chair);
      rect(item.x, item.y, item.width, 54, 16, furniture.wood);
      rect(item.x + 58, item.y + 12, 48, 26, 5, furniture.screen);
      ellipse(item.x + 25, item.y + 30, 8, 8, furniture.leaf);
    } else if (item.kind === 'table') {
      rect(item.x + 24, item.y, 62, 35, 14, furniture.seat);
      rect(item.x + 24, item.y + 112, 62, 35, 14, furniture.seat);
      ellipse(item.x + 56, item.y + 74, 54, 54, furniture.wood);
      ellipse(item.x + 56, item.y + 74, 11, 11, furniture.leaf);
    } else if (item.kind === 'sofa') {
      rect(item.x, item.y, item.width, item.depth, 18, furniture.sofa);
      if (item.width > item.depth)
        rect(item.x + 8, item.y + item.depth - 16, item.width - 16, 10, 5, furniture.cushion);
      else rect(item.x + 6, item.y + 8, 12, item.depth - 16, 5, furniture.cushion);
    } else {
      ellipse(item.x + 20, item.y + 26, 16, 14, furniture.pot);
      ellipse(item.x + 13, item.y + 12, 8, 14, furniture.leaf);
      ellipse(item.x + 28, item.y + 11, 7, 12, furniture.leaf);
    }
  }
  return shapes;
}
