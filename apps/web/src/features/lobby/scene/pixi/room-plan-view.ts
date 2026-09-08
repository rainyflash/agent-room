import type { Graphics } from 'pixi.js';
import type { LobbyWorld } from '../../domain/scene-projection';
import { roomPlanShapes } from '../room-plan-art';

export function drawRoomPlan(graphic: Graphics, world: LobbyWorld): void {
  graphic.clear();
  for (const shape of roomPlanShapes(world)) {
    if (shape.kind === 'line') {
      graphic
        .moveTo(shape.x, shape.y)
        .lineTo(shape.toX, shape.toY)
        .stroke({ color: shape.stroke, width: 2 });
    } else {
      if (shape.kind === 'ellipse') graphic.ellipse(shape.x, shape.y, shape.rx, shape.ry);
      else graphic.roundRect(shape.x, shape.y, shape.width, shape.height, shape.radius);
      graphic.fill(shape.fill);
      if (shape.stroke !== 'transparent') graphic.stroke({ color: shape.stroke, width: 2 });
    }
  }
}
