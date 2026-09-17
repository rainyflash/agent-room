import type { Graphics } from 'pixi.js';
import type { LobbyWorld } from '../../domain/scene-projection';
import { roomPlanShapes } from '../room-plan-art';

/** 地砖不在这里画：棋盘格由场景用一张平铺纹理铺满，见 pixi-lobby-scene.ts。 */
export function drawRoomPlan(graphic: Graphics, world: LobbyWorld): void {
  graphic.clear();
  for (const shape of roomPlanShapes(world)) {
    if (shape.kind === 'floor') continue;
    if (shape.kind === 'line') {
      graphic
        .moveTo(shape.x, shape.y)
        .lineTo(shape.toX, shape.toY)
        .stroke({ color: shape.stroke, width: shape.strokeWidth, cap: 'round' });
    } else {
      if (shape.kind === 'ellipse') graphic.ellipse(shape.x, shape.y, shape.rx, shape.ry);
      else graphic.roundRect(shape.x, shape.y, shape.width, shape.height, shape.radius);
      graphic.fill(shape.fill).stroke({ color: shape.stroke, width: shape.strokeWidth });
    }
  }
}
