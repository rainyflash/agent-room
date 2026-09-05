import type { Graphics } from 'pixi.js';
import type { SceneShape } from '../room-art';

export function shapeGraphics(
  pixi: typeof import('pixi.js'),
  shapes: readonly SceneShape[],
): Graphics {
  const graphics = new pixi.Graphics();
  for (const shape of shapes) {
    switch (shape.kind) {
      case 'polygon':
        graphics.poly([...shape.points]);
        break;
      case 'ellipse':
        graphics.ellipse(shape.x, shape.y, shape.rx, shape.ry);
        break;
      case 'rect':
        graphics.roundRect(shape.x, shape.y, shape.width, shape.height, shape.radius);
        break;
    }
    graphics.fill(shape.fill);
    if (shape.stroke !== undefined) graphics.stroke({ color: shape.stroke, width: 1.5 });
  }
  graphics.eventMode = 'none';
  return graphics;
}
